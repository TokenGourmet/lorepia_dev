use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex, TryLockError,
};
use std::time::{Duration, Instant};

use mlua::chunk::ChunkMode;
use mlua::{Error as LuaError, HookTriggers, Lua, LuaOptions, StdLib, Value, VmState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PROTOCOL_VERSION: u8 = 1;
const POLICY_VERSION: &str = "m1-lua-limits-v1";
const MLUA_VERSION: &str = "0.12.0";
const LUA_VERSION: &str = "Lua 5.4";
const FIXTURE_CATALOG_SHA256: &str =
    "9ea567d6901ec39412e73f439ee9ea7d47538baea4d1a92cd409c9f3e9b97db5";

const DEADLINE_MS: u64 = 50;
const INSTRUCTION_CAP: u64 = 100_000;
const HOOK_CADENCE: u32 = 1_000;
const MEMORY_CEILING_BYTES: usize = 8 * 1024 * 1024;
const MAX_SERIALIZED_BYTES: usize = 4_096;

const DEADLINE_MARKER: &str = "__LOREPIA_M1_DEADLINE_LIMIT__";
const INSTRUCTION_MARKER: &str = "__LOREPIA_M1_INSTRUCTION_LIMIT__";

const ALLOWED_GLOBALS: [&str; 4] = ["math", "string", "table", "utf8"];
const FORBIDDEN_GLOBALS: [&str; 14] = [
    "os",
    "io",
    "package",
    "debug",
    "require",
    "dofile",
    "loadfile",
    "load",
    "collectgarbage",
    "pcall",
    "xpcall",
    "coroutine",
    "print",
    "warn",
];
const BYPASS_GLOBALS: [&str; 6] = ["pcall", "xpcall", "coroutine", "package", "debug", "load"];

const FIXTURE_CATALOG: &str = include_str!("../../fixtures/catalog.json");
const ALLOWED_SOURCE: &str = include_str!("../../fixtures/allowed.lua");
const INFINITE_LOOP_SOURCE: &str = include_str!("../../fixtures/infinite-loop.lua");
const RECURSIVE_PRESSURE_SOURCE: &str = include_str!("../../fixtures/recursive-pressure.lua");
const ALLOCATOR_PRESSURE_SOURCE: &str = include_str!("../../fixtures/allocator-pressure.lua");
const FORBIDDEN_GLOBALS_SOURCE: &str = include_str!("../../fixtures/forbidden-globals.lua");
const BYPASS_SURFACES_SOURCE: &str = include_str!("../../fixtures/bypass-surfaces.lua");

static PROBE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureCatalog {
    protocol_version: String,
    policy_version: String,
    origin: String,
    license: String,
    fixtures: Vec<FixtureRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureRecord {
    fixture_id: String,
    file: String,
    source_bytes: usize,
    source_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CaseOutcome {
    Allowed,
    Interrupted,
    Absent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CaseCode {
    AllowedResult,
    InstructionLimit,
    DeadlineLimit,
    StackLimit,
    MemoryLimit,
    ForbiddenGlobalsAbsent,
    BypassSurfacesAbsent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ProbeErrorCode {
    ProbeBusy,
    ProbeContractFailed,
    InternalState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeError {
    protocol_version: u8,
    code: ProbeErrorCode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReceipt {
    protocol_version: u8,
    policy_version: &'static str,
    fixture_catalog_sha256: String,
    mlua_version: &'static str,
    lua_version: &'static str,
    limits: ProbeLimits,
    cases: Vec<CaseReceipt>,
    defenses: ProbeDefenses,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeLimits {
    deadline_ms: u64,
    instruction_cap: u64,
    hook_cadence: u32,
    memory_ceiling_bytes: usize,
    max_serialized_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeDefenses {
    fresh_state_per_case: bool,
    text_mode_only: bool,
    forbidden_globals_absent: bool,
    bypass_surfaces_absent: bool,
    process_lock_nonblocking: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseReceipt {
    case_id: &'static str,
    outcome: CaseOutcome,
    code: CaseCode,
    result: Option<i64>,
    elapsed_micros: u64,
    hook_count: u64,
    instruction_estimate: u64,
    used_memory_bytes: usize,
    observed_peak_memory_bytes: usize,
    memory_ceiling_bytes: usize,
}

#[derive(Clone, Copy)]
enum CaseKind {
    Allowed,
    InfiniteLoop,
    RecursivePressure,
    AllocatorPressure,
    ForbiddenGlobals,
    BypassSurfaces,
}

enum ExecutionValue {
    Integer(i64),
    Boolean(bool),
    Completed,
}

struct HookMetrics {
    started: Instant,
    hook_count: AtomicU64,
    observed_peak_memory: AtomicUsize,
}

impl HookMetrics {
    fn new(initial_memory: usize) -> Self {
        Self {
            started: Instant::now(),
            hook_count: AtomicU64::new(0),
            observed_peak_memory: AtomicUsize::new(initial_memory),
        }
    }
}

impl ProbeError {
    fn new(code: ProbeErrorCode) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            code,
        }
    }
}

pub(crate) fn internal_state_error() -> ProbeError {
    ProbeError::new(ProbeErrorCode::InternalState)
}

fn contract_error() -> ProbeError {
    ProbeError::new(ProbeErrorCode::ProbeContractFailed)
}

fn busy_error() -> ProbeError {
    ProbeError::new(ProbeErrorCode::ProbeBusy)
}

pub(crate) fn run_probe() -> Result<ProbeReceipt, ProbeError> {
    with_process_lock(run_probe_unlocked)
}

fn with_process_lock<T>(task: impl FnOnce() -> Result<T, ProbeError>) -> Result<T, ProbeError> {
    let _guard = match PROBE_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => return Err(busy_error()),
        Err(TryLockError::Poisoned(_)) => return Err(internal_state_error()),
    };
    task()
}

fn run_probe_unlocked() -> Result<ProbeReceipt, ProbeError> {
    validate_fixture_bundle()?;

    let cases = vec![
        execute_case("allowed-baseline", ALLOWED_SOURCE, CaseKind::Allowed)?,
        execute_case(
            "infinite-loop",
            INFINITE_LOOP_SOURCE,
            CaseKind::InfiniteLoop,
        )?,
        execute_case(
            "recovery-after-infinite-loop",
            ALLOWED_SOURCE,
            CaseKind::Allowed,
        )?,
        execute_case(
            "recursive-pressure",
            RECURSIVE_PRESSURE_SOURCE,
            CaseKind::RecursivePressure,
        )?,
        execute_case(
            "recovery-after-recursive-pressure",
            ALLOWED_SOURCE,
            CaseKind::Allowed,
        )?,
        execute_case(
            "allocator-pressure",
            ALLOCATOR_PRESSURE_SOURCE,
            CaseKind::AllocatorPressure,
        )?,
        execute_case(
            "recovery-after-allocator-pressure",
            ALLOWED_SOURCE,
            CaseKind::Allowed,
        )?,
        execute_case(
            "forbidden-globals-absent",
            FORBIDDEN_GLOBALS_SOURCE,
            CaseKind::ForbiddenGlobals,
        )?,
        execute_case(
            "recovery-after-forbidden-globals",
            ALLOWED_SOURCE,
            CaseKind::Allowed,
        )?,
        execute_case(
            "bypass-surfaces-absent",
            BYPASS_SURFACES_SOURCE,
            CaseKind::BypassSurfaces,
        )?,
        execute_case(
            "recovery-after-bypass-surfaces",
            ALLOWED_SOURCE,
            CaseKind::Allowed,
        )?,
    ];

    let receipt = ProbeReceipt {
        protocol_version: PROTOCOL_VERSION,
        policy_version: POLICY_VERSION,
        fixture_catalog_sha256: FIXTURE_CATALOG_SHA256.to_owned(),
        mlua_version: MLUA_VERSION,
        lua_version: LUA_VERSION,
        limits: ProbeLimits {
            deadline_ms: DEADLINE_MS,
            instruction_cap: INSTRUCTION_CAP,
            hook_cadence: HOOK_CADENCE,
            memory_ceiling_bytes: MEMORY_CEILING_BYTES,
            max_serialized_bytes: MAX_SERIALIZED_BYTES,
        },
        cases,
        defenses: ProbeDefenses {
            fresh_state_per_case: true,
            text_mode_only: true,
            forbidden_globals_absent: true,
            bypass_surfaces_absent: true,
            process_lock_nonblocking: true,
        },
    };

    let serialized = serde_json::to_vec(&receipt).map_err(|_| internal_state_error())?;
    if serialized.len() > MAX_SERIALIZED_BYTES {
        return Err(contract_error());
    }

    Ok(receipt)
}

fn validate_fixture_bundle() -> Result<(), ProbeError> {
    if sha256_hex(FIXTURE_CATALOG.as_bytes()) != FIXTURE_CATALOG_SHA256 {
        return Err(contract_error());
    }

    let catalog: FixtureCatalog =
        serde_json::from_str(FIXTURE_CATALOG).map_err(|_| contract_error())?;
    if catalog.protocol_version != "m1-lua-limits-fixtures-v1"
        || catalog.policy_version != POLICY_VERSION
        || catalog.origin != "self-authored"
        || catalog.license != "CC0-1.0"
    {
        return Err(contract_error());
    }

    let expected = [
        ("allowed", "allowed.lua", ALLOWED_SOURCE),
        ("infinite-loop", "infinite-loop.lua", INFINITE_LOOP_SOURCE),
        (
            "recursive-pressure",
            "recursive-pressure.lua",
            RECURSIVE_PRESSURE_SOURCE,
        ),
        (
            "allocator-pressure",
            "allocator-pressure.lua",
            ALLOCATOR_PRESSURE_SOURCE,
        ),
        (
            "forbidden-globals",
            "forbidden-globals.lua",
            FORBIDDEN_GLOBALS_SOURCE,
        ),
        (
            "bypass-surfaces",
            "bypass-surfaces.lua",
            BYPASS_SURFACES_SOURCE,
        ),
    ];
    if catalog.fixtures.len() != expected.len() {
        return Err(contract_error());
    }

    for (record, (fixture_id, file, source)) in catalog.fixtures.iter().zip(expected) {
        if record.fixture_id != fixture_id
            || record.file != file
            || record.source_bytes != source.len()
            || record.source_sha256 != sha256_hex(source.as_bytes())
        {
            return Err(contract_error());
        }
    }
    Ok(())
}

fn execute_case(
    case_id: &'static str,
    source: &'static str,
    kind: CaseKind,
) -> Result<CaseReceipt, ProbeError> {
    let lua = new_lua_state()?;

    match kind {
        CaseKind::ForbiddenGlobals => verify_globals_absent(&lua, &FORBIDDEN_GLOBALS)?,
        CaseKind::BypassSurfaces => verify_globals_absent(&lua, &BYPASS_GLOBALS)?,
        _ => {}
    }

    let metrics = Arc::new(HookMetrics::new(lua.used_memory()));
    install_hook(&lua, Arc::clone(&metrics))?;

    let execution = match kind {
        CaseKind::Allowed => lua
            .load(source)
            .set_mode(ChunkMode::Text)
            .eval::<i64>()
            .map(ExecutionValue::Integer),
        CaseKind::ForbiddenGlobals | CaseKind::BypassSurfaces => lua
            .load(source)
            .set_mode(ChunkMode::Text)
            .eval::<bool>()
            .map(ExecutionValue::Boolean),
        CaseKind::InfiniteLoop | CaseKind::RecursivePressure | CaseKind::AllocatorPressure => lua
            .load(source)
            .set_mode(ChunkMode::Text)
            .exec()
            .map(|()| ExecutionValue::Completed),
    };

    let elapsed = metrics.started.elapsed();
    lua.remove_hook();
    let hook_count = metrics.hook_count.load(Ordering::Relaxed);
    let used_memory = lua.used_memory();
    metrics
        .observed_peak_memory
        .fetch_max(used_memory, Ordering::Relaxed);
    let observed_peak_memory = metrics.observed_peak_memory.load(Ordering::Relaxed);
    let instruction_estimate = hook_count.saturating_mul(u64::from(HOOK_CADENCE));

    let (outcome, code, result) = evaluate_case_result(kind, execution, elapsed)?;

    Ok(CaseReceipt {
        case_id,
        outcome,
        code,
        result,
        elapsed_micros: duration_micros(elapsed),
        hook_count,
        instruction_estimate,
        used_memory_bytes: used_memory,
        observed_peak_memory_bytes: observed_peak_memory,
        memory_ceiling_bytes: MEMORY_CEILING_BYTES,
    })
}

fn evaluate_case_result(
    kind: CaseKind,
    execution: Result<ExecutionValue, LuaError>,
    elapsed: Duration,
) -> Result<(CaseOutcome, CaseCode, Option<i64>), ProbeError> {
    match (kind, execution) {
        (CaseKind::Allowed, Ok(ExecutionValue::Integer(55)))
            if elapsed <= Duration::from_millis(DEADLINE_MS) =>
        {
            Ok((CaseOutcome::Allowed, CaseCode::AllowedResult, Some(55)))
        }
        (CaseKind::ForbiddenGlobals, Ok(ExecutionValue::Boolean(true)))
            if elapsed <= Duration::from_millis(DEADLINE_MS) =>
        {
            Ok((CaseOutcome::Absent, CaseCode::ForbiddenGlobalsAbsent, None))
        }
        (CaseKind::BypassSurfaces, Ok(ExecutionValue::Boolean(true)))
            if elapsed <= Duration::from_millis(DEADLINE_MS) =>
        {
            Ok((CaseOutcome::Absent, CaseCode::BypassSurfacesAbsent, None))
        }
        (CaseKind::InfiniteLoop, Err(error)) => {
            let code = classify_interrupt(&error).ok_or_else(contract_error)?;
            if matches!(code, CaseCode::InstructionLimit | CaseCode::DeadlineLimit) {
                Ok((CaseOutcome::Interrupted, code, None))
            } else {
                Err(contract_error())
            }
        }
        (CaseKind::RecursivePressure, Err(error)) => {
            let code = classify_interrupt(&error).ok_or_else(contract_error)?;
            if matches!(
                code,
                CaseCode::StackLimit | CaseCode::InstructionLimit | CaseCode::DeadlineLimit
            ) {
                Ok((CaseOutcome::Interrupted, code, None))
            } else {
                Err(contract_error())
            }
        }
        (CaseKind::AllocatorPressure, Err(error)) => {
            let code = classify_interrupt(&error).ok_or_else(contract_error)?;
            if code == CaseCode::MemoryLimit {
                Ok((CaseOutcome::Interrupted, code, None))
            } else {
                Err(contract_error())
            }
        }
        _ => Err(contract_error()),
    }
}

fn new_lua_state() -> Result<Lua, ProbeError> {
    let selected_libraries = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8;
    let lua =
        Lua::new_with(selected_libraries, LuaOptions::new()).map_err(|_| internal_state_error())?;

    if lua.used_memory() > MEMORY_CEILING_BYTES {
        return Err(contract_error());
    }
    let previous_limit = lua
        .set_memory_limit(MEMORY_CEILING_BYTES)
        .map_err(|_| internal_state_error())?;
    if previous_limit != 0 {
        return Err(internal_state_error());
    }

    let globals = lua.globals();
    let runtime_version: String = globals
        .get("_VERSION")
        .map_err(|_| internal_state_error())?;
    if runtime_version != LUA_VERSION {
        return Err(contract_error());
    }

    let global_keys = globals
        .clone()
        .pairs::<String, Value>()
        .map(|entry| entry.map(|(key, _)| key))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| internal_state_error())?;
    for key in global_keys {
        if !ALLOWED_GLOBALS.contains(&key.as_str()) {
            globals
                .set(key, Value::Nil)
                .map_err(|_| internal_state_error())?;
        }
    }

    verify_exact_global_surface(&lua)?;
    Ok(lua)
}

fn verify_exact_global_surface(lua: &Lua) -> Result<(), ProbeError> {
    let globals = lua.globals();
    let mut actual_keys = globals
        .clone()
        .pairs::<String, Value>()
        .map(|entry| entry.map(|(key, _)| key))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| internal_state_error())?;
    actual_keys.sort_unstable();

    if actual_keys != ALLOWED_GLOBALS {
        return Err(contract_error());
    }
    for key in ALLOWED_GLOBALS {
        let value: Value = globals.get(key).map_err(|_| internal_state_error())?;
        if !matches!(value, Value::Table(_)) {
            return Err(contract_error());
        }
    }
    Ok(())
}

fn verify_globals_absent(lua: &Lua, names: &[&str]) -> Result<(), ProbeError> {
    let globals = lua.globals();
    for name in names {
        let value: Value = globals.get(*name).map_err(|_| internal_state_error())?;
        if !matches!(value, Value::Nil) {
            return Err(contract_error());
        }
    }
    Ok(())
}

fn install_hook(lua: &Lua, metrics: Arc<HookMetrics>) -> Result<(), ProbeError> {
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_CADENCE),
        move |lua, _debug| {
            let hook_count = metrics.hook_count.fetch_add(1, Ordering::Relaxed) + 1;
            metrics
                .observed_peak_memory
                .fetch_max(lua.used_memory(), Ordering::Relaxed);

            if metrics.started.elapsed() >= Duration::from_millis(DEADLINE_MS) {
                return Err(LuaError::runtime(DEADLINE_MARKER));
            }
            if hook_count.saturating_mul(u64::from(HOOK_CADENCE)) >= INSTRUCTION_CAP {
                return Err(LuaError::runtime(INSTRUCTION_MARKER));
            }
            Ok(VmState::Continue)
        },
    )
    .map_err(|_| internal_state_error())
}

fn classify_interrupt(error: &LuaError) -> Option<CaseCode> {
    if error_chain_matches(error, |current| matches!(current, LuaError::MemoryError(_))) {
        return Some(CaseCode::MemoryLimit);
    }
    if error_chain_matches(
        error,
        |current| matches!(current, LuaError::RuntimeError(message) if message == DEADLINE_MARKER),
    ) {
        return Some(CaseCode::DeadlineLimit);
    }
    if error_chain_matches(
        error,
        |current| matches!(current, LuaError::RuntimeError(message) if message == INSTRUCTION_MARKER),
    ) {
        return Some(CaseCode::InstructionLimit);
    }
    if error_chain_matches(error, |current| {
        matches!(current, LuaError::StackError)
            || matches!(current, LuaError::RuntimeError(message) if message.contains("stack overflow"))
    }) {
        return Some(CaseCode::StackLimit);
    }
    None
}

fn error_chain_matches(error: &LuaError, predicate: impl Fn(&LuaError) -> bool + Copy) -> bool {
    if predicate(error) {
        return true;
    }
    match error {
        LuaError::BadArgument { cause, .. }
        | LuaError::CallbackError { cause, .. }
        | LuaError::WithContext { cause, .. } => error_chain_matches(cause, predicate),
        _ => false,
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable_projection(receipt: &ProbeReceipt) -> Vec<(&'static str, CaseOutcome, Option<i64>)> {
        receipt
            .cases
            .iter()
            .map(|case| (case.case_id, case.outcome, case.result))
            .collect()
    }

    #[test]
    fn policy_constants_and_global_surface_are_exact() {
        assert_eq!(POLICY_VERSION, "m1-lua-limits-v1");
        assert_eq!(DEADLINE_MS, 50);
        assert_eq!(INSTRUCTION_CAP, 100_000);
        assert_eq!(HOOK_CADENCE, 1_000);
        assert_eq!(MEMORY_CEILING_BYTES, 8_388_608);
        assert_eq!(MAX_SERIALIZED_BYTES, 4_096);
        assert_eq!(ALLOWED_GLOBALS, ["math", "string", "table", "utf8"]);

        let lua = new_lua_state().expect("sandbox state should initialize");
        verify_exact_global_surface(&lua).expect("global surface should stay exact");
        verify_globals_absent(&lua, &FORBIDDEN_GLOBALS)
            .expect("forbidden globals should be absent");
    }

    #[test]
    fn fixture_catalog_order_sizes_and_hashes_are_pinned() {
        validate_fixture_bundle().expect("production catalog validation should pass");
        let catalog: FixtureCatalog =
            serde_json::from_str(FIXTURE_CATALOG).expect("catalog must parse");
        assert_eq!(catalog.protocol_version, "m1-lua-limits-fixtures-v1");
        assert_eq!(catalog.policy_version, POLICY_VERSION);
        assert_eq!(catalog.origin, "self-authored");
        assert_eq!(catalog.license, "CC0-1.0");
        assert_eq!(
            sha256_hex(FIXTURE_CATALOG.as_bytes()),
            FIXTURE_CATALOG_SHA256
        );

        let expected = [
            ("allowed", "allowed.lua", ALLOWED_SOURCE),
            ("infinite-loop", "infinite-loop.lua", INFINITE_LOOP_SOURCE),
            (
                "recursive-pressure",
                "recursive-pressure.lua",
                RECURSIVE_PRESSURE_SOURCE,
            ),
            (
                "allocator-pressure",
                "allocator-pressure.lua",
                ALLOCATOR_PRESSURE_SOURCE,
            ),
            (
                "forbidden-globals",
                "forbidden-globals.lua",
                FORBIDDEN_GLOBALS_SOURCE,
            ),
            (
                "bypass-surfaces",
                "bypass-surfaces.lua",
                BYPASS_SURFACES_SOURCE,
            ),
        ];
        assert_eq!(catalog.fixtures.len(), expected.len());
        for (record, (fixture_id, file, source)) in catalog.fixtures.iter().zip(expected) {
            assert_eq!(record.fixture_id, fixture_id);
            assert_eq!(record.file, file);
            assert_eq!(record.source_bytes, source.len());
            assert_eq!(record.source_sha256, sha256_hex(source.as_bytes()));
        }
    }

    #[test]
    fn hostile_cases_interrupt_and_every_followup_state_recovers() {
        let receipt = run_probe_unlocked().expect("probe should pass");
        let expected_order = [
            "allowed-baseline",
            "infinite-loop",
            "recovery-after-infinite-loop",
            "recursive-pressure",
            "recovery-after-recursive-pressure",
            "allocator-pressure",
            "recovery-after-allocator-pressure",
            "forbidden-globals-absent",
            "recovery-after-forbidden-globals",
            "bypass-surfaces-absent",
            "recovery-after-bypass-surfaces",
        ];
        assert_eq!(
            receipt
                .cases
                .iter()
                .map(|case| case.case_id)
                .collect::<Vec<_>>(),
            expected_order
        );

        for index in [0_usize, 2, 4, 6, 8, 10] {
            let recovery = &receipt.cases[index];
            assert_eq!(recovery.outcome, CaseOutcome::Allowed);
            assert_eq!(recovery.code, CaseCode::AllowedResult);
            assert_eq!(recovery.result, Some(55));
        }
        assert!(matches!(
            receipt.cases[1].code,
            CaseCode::InstructionLimit | CaseCode::DeadlineLimit
        ));
        assert!(matches!(
            receipt.cases[3].code,
            CaseCode::StackLimit | CaseCode::InstructionLimit | CaseCode::DeadlineLimit
        ));
        assert_eq!(receipt.cases[5].code, CaseCode::MemoryLimit);
        assert_eq!(receipt.cases[7].code, CaseCode::ForbiddenGlobalsAbsent);
        assert_eq!(receipt.cases[9].code, CaseCode::BypassSurfacesAbsent);
        for case in &receipt.cases {
            assert_eq!(case.memory_ceiling_bytes, MEMORY_CEILING_BYTES);
            assert_eq!(
                case.instruction_estimate,
                case.hook_count.saturating_mul(u64::from(HOOK_CADENCE))
            );
            assert!(case.used_memory_bytes <= case.observed_peak_memory_bytes);
            assert!(case.observed_peak_memory_bytes <= MEMORY_CEILING_BYTES);
        }
    }

    #[test]
    fn stable_receipt_projection_is_deterministic() {
        let first = run_probe_unlocked().expect("first probe should pass");
        let second = run_probe_unlocked().expect("second probe should pass");
        assert_eq!(first.protocol_version, second.protocol_version);
        assert_eq!(first.policy_version, second.policy_version);
        assert_eq!(first.fixture_catalog_sha256, second.fixture_catalog_sha256);
        assert_eq!(first.limits, second.limits);
        assert_eq!(first.defenses, second.defenses);
        assert_eq!(stable_projection(&first), stable_projection(&second));
    }

    #[test]
    fn serialized_receipt_is_bounded_and_contains_no_raw_source_or_error() {
        let receipt = run_probe_unlocked().expect("probe should pass");
        let encoded = serde_json::to_string(&receipt).expect("receipt should serialize");
        assert!(encoded.len() <= MAX_SERIALIZED_BYTES);
        for prohibited in [
            "while true",
            "local function descend",
            "stack overflow",
            "memory error",
            DEADLINE_MARKER,
            INSTRUCTION_MARKER,
            ".lua",
            "/Users/",
            "/tmp/",
        ] {
            assert!(!encoded.contains(prohibited));
        }
    }

    #[test]
    fn serialized_success_json_surface_is_exact() {
        let receipt = run_probe_unlocked().expect("probe should pass");
        let value = serde_json::to_value(receipt).expect("receipt should serialize");
        let object = value.as_object().expect("receipt should be an object");
        let mut top_level_keys = object.keys().map(String::as_str).collect::<Vec<_>>();
        top_level_keys.sort_unstable();
        assert_eq!(
            top_level_keys,
            [
                "cases",
                "defenses",
                "fixtureCatalogSha256",
                "limits",
                "luaVersion",
                "mluaVersion",
                "policyVersion",
                "protocolVersion",
            ]
        );
        assert_eq!(
            object
                .get("protocolVersion")
                .and_then(|value| value.as_u64()),
            Some(u64::from(PROTOCOL_VERSION))
        );

        let limits = object
            .get("limits")
            .and_then(|value| value.as_object())
            .expect("limits should be an object");
        let mut limit_keys = limits.keys().map(String::as_str).collect::<Vec<_>>();
        limit_keys.sort_unstable();
        assert_eq!(
            limit_keys,
            [
                "deadlineMs",
                "hookCadence",
                "instructionCap",
                "maxSerializedBytes",
                "memoryCeilingBytes",
            ]
        );

        let defenses = object
            .get("defenses")
            .and_then(|value| value.as_object())
            .expect("defenses should be an object");
        let mut defense_keys = defenses.keys().map(String::as_str).collect::<Vec<_>>();
        defense_keys.sort_unstable();
        assert_eq!(
            defense_keys,
            [
                "bypassSurfacesAbsent",
                "forbiddenGlobalsAbsent",
                "freshStatePerCase",
                "processLockNonblocking",
                "textModeOnly",
            ]
        );

        let first_case = object
            .get("cases")
            .and_then(|value| value.as_array())
            .and_then(|cases| cases.first())
            .and_then(|value| value.as_object())
            .expect("first case should be an object");
        let mut case_keys = first_case.keys().map(String::as_str).collect::<Vec<_>>();
        case_keys.sort_unstable();
        assert_eq!(
            case_keys,
            [
                "caseId",
                "code",
                "elapsedMicros",
                "hookCount",
                "instructionEstimate",
                "memoryCeilingBytes",
                "observedPeakMemoryBytes",
                "outcome",
                "result",
                "usedMemoryBytes",
            ]
        );
    }

    #[test]
    fn process_lock_rejects_concurrent_entry_without_waiting() {
        let guard = PROBE_LOCK.lock().expect("test lock should be available");
        let error = with_process_lock(|| Ok(())).expect_err("second entry must be busy");
        assert_eq!(error.code, ProbeErrorCode::ProbeBusy);
        drop(guard);
    }

    #[test]
    fn failure_json_has_only_protocol_and_bounded_code() {
        for (error, expected_code) in [
            (busy_error(), "PROBE_BUSY"),
            (contract_error(), "PROBE_CONTRACT_FAILED"),
            (internal_state_error(), "INTERNAL_STATE"),
        ] {
            let value = serde_json::to_value(error).expect("error should serialize");
            let object = value.as_object().expect("error should be an object");
            assert_eq!(object.len(), 2);
            assert_eq!(
                object
                    .get("protocolVersion")
                    .and_then(|value| value.as_u64()),
                Some(u64::from(PROTOCOL_VERSION))
            );
            assert_eq!(
                object.get("code").and_then(|value| value.as_str()),
                Some(expected_code)
            );
        }
    }

    #[test]
    fn pcall_xpcall_and_coroutine_bypass_surfaces_are_really_absent() {
        let lua = new_lua_state().expect("sandbox state should initialize");
        verify_globals_absent(&lua, &BYPASS_GLOBALS).expect("bypass globals must be absent");
        let result = lua
            .load(BYPASS_SURFACES_SOURCE)
            .set_mode(ChunkMode::Text)
            .eval::<bool>()
            .expect("fixed bypass fixture should execute");
        assert!(result);
    }

    #[test]
    fn text_mode_rejects_a_precompiled_chunk_marker_before_execution() {
        let lua = new_lua_state().expect("sandbox state should initialize");
        let binary_marker = b"\x1bLua\x54\x00\x19\x93\r\n\x1a\n";
        let result = lua
            .load(binary_marker.as_slice())
            .set_mode(ChunkMode::Text)
            .into_function();
        assert!(result.is_err());
    }

    #[test]
    fn cargo_feature_surface_is_lua54_vendored_only() {
        const CARGO_TOML: &str = include_str!("../Cargo.toml");
        const CARGO_LOCK: &str = include_str!("../Cargo.lock");
        let mlua_line = CARGO_TOML
            .lines()
            .find(|line| line.starts_with("mlua = "))
            .expect("mlua dependency must exist");
        assert_eq!(
            mlua_line,
            "mlua = { version = \"=0.12.0\", default-features = false, features = [\"lua54\", \"vendored\"] }"
        );
        for forbidden_feature in ["luajit", "module", "async", "send"] {
            assert!(!mlua_line.contains(forbidden_feature));
        }
        assert!(CARGO_LOCK.contains("name = \"mlua\"\nversion = \"0.12.0\""));
    }
}

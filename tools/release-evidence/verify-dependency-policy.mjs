import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

function requireString(value, field) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`MISSING_${field.toUpperCase()}`);
  }
  return value;
}

function allowedSet(policy, ecosystem) {
  const expressions = policy?.[ecosystem]?.allowedLicenseExpressions;
  if (!Array.isArray(expressions) || expressions.length === 0) {
    throw new Error(`EMPTY_${ecosystem.toUpperCase()}_LICENSE_POLICY`);
  }
  if (expressions.some((expression) => typeof expression !== 'string' || expression.length === 0)) {
    throw new Error(`INVALID_${ecosystem.toUpperCase()}_LICENSE_POLICY`);
  }
  return new Set(expressions);
}

export function verifyDependencyPolicy({ cargoMetadata, npmLock, policy }) {
  if (policy?.schemaVersion !== 1) throw new Error('UNSUPPORTED_POLICY_VERSION');

  const cargoAllowed = allowedSet(policy, 'cargo');
  const npmAllowed = allowedSet(policy, 'npm');
  const cargoPackages = Array.isArray(cargoMetadata?.packages) ? cargoMetadata.packages : null;
  if (cargoPackages === null) throw new Error('INVALID_CARGO_METADATA');

  const violations = [];
  for (const dependency of cargoPackages) {
    const name = requireString(dependency?.name, 'cargo_package_name');
    const version = requireString(dependency?.version, 'cargo_package_version');
    const license = dependency?.license;
    if (typeof license !== 'string' || !cargoAllowed.has(license)) {
      violations.push({ ecosystem: 'cargo', name, version, license: license ?? null });
    }
  }

  const npmPackages = npmLock?.packages;
  if (npmPackages === null || typeof npmPackages !== 'object' || Array.isArray(npmPackages)) {
    throw new Error('INVALID_NPM_LOCK');
  }
  for (const [packagePath, dependency] of Object.entries(npmPackages)) {
    if (packagePath === '') continue;
    const name = dependency?.name ?? packagePath.replace(/^node_modules\//u, '');
    const version = requireString(dependency?.version, 'npm_package_version');
    const license = dependency?.license;
    if (typeof license !== 'string' || !npmAllowed.has(license)) {
      violations.push({ ecosystem: 'npm', name, version, license: license ?? null });
    }
  }

  violations.sort((left, right) =>
    `${left.ecosystem}:${left.name}:${left.version}`.localeCompare(
      `${right.ecosystem}:${right.name}:${right.version}`,
    ),
  );
  if (violations.length > 0) {
    throw new Error(`DEPENDENCY_LICENSE_REVIEW_REQUIRED:${JSON.stringify(violations)}`);
  }

  return { cargoPackages: cargoPackages.length, npmPackages: Object.keys(npmPackages).length - 1 };
}

async function main() {
  const [cargoPath, npmPath, policyPath] = process.argv.slice(2);
  if (!cargoPath || !npmPath || !policyPath) {
    throw new Error('USAGE: verify-dependency-policy <cargo-metadata.json> <package-lock.json> <policy.json>');
  }
  const [cargoMetadata, npmLock, policy] = await Promise.all(
    [cargoPath, npmPath, policyPath].map(async (file) => JSON.parse(await readFile(file, 'utf8'))),
  );
  const result = verifyDependencyPolicy({ cargoMetadata, npmLock, policy });
  process.stdout.write(
    `dependency policy PASS: ${result.cargoPackages} Cargo packages, ${result.npmPackages} npm packages\n`,
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}

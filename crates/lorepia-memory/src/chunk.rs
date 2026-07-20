use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    ChunkSeparatorRegex, MAX_MEMORY_ARTIFACT_BYTES, MAX_MEMORY_CHUNKS, MemoryArtifactId,
    MemoryError, Result,
};

const CHUNK_ID_DOMAIN: &[u8] = b"lorepia-memory-chunk-v1";

/// One deterministic, non-executable slice of a parent memory artifact.
///
/// This type is intentionally serialize-only. Portable data cannot construct
/// chunks with caller-selected identifiers; chunks must come from
/// [`split_memory_chunks`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryChunk {
    id: MemoryArtifactId,
    parent_artifact_id: MemoryArtifactId,
    index: u32,
    content: String,
}

impl MemoryChunk {
    #[must_use]
    pub const fn id(&self) -> &MemoryArtifactId {
        &self.id
    }

    #[must_use]
    pub const fn parent_artifact_id(&self) -> &MemoryArtifactId {
        &self.parent_artifact_id
    }

    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// Split bounded artifact text with the preset's pinned Rust regex.
///
/// Separators are removed, empty or whitespace-only segments are ignored, and
/// every retained slice is otherwise preserved exactly. No segment is
/// interpreted as a template or executable input. Chunk identifiers are
/// SHA-256 digests over a domain separator, the parent artifact ID, the retained
/// chunk index, and the exact chunk bytes.
pub fn split_memory_chunks(
    parent_artifact_id: &MemoryArtifactId,
    content: &str,
    separator: &ChunkSeparatorRegex,
) -> Result<Vec<MemoryChunk>> {
    validate_input(content)?;
    let separator = separator.compile()?;
    let mut chunks = Vec::new();
    let mut segment_start = 0usize;

    for matched in separator.find_iter(content) {
        // `ChunkSeparatorRegex::new` rejects patterns that match an empty
        // haystack, but assertions such as a word boundary can still produce
        // an empty match only for particular runtime input.
        if matched.start() == matched.end() {
            return Err(MemoryError::Regex(
                "separator produced a zero-width runtime match",
            ));
        }
        push_non_empty_chunk(
            &mut chunks,
            parent_artifact_id,
            &content[segment_start..matched.start()],
        )?;
        segment_start = matched.end();
    }

    push_non_empty_chunk(&mut chunks, parent_artifact_id, &content[segment_start..])?;
    if chunks.is_empty() {
        return Err(MemoryError::invalid(
            "memoryChunkInput",
            "must contain at least one non-whitespace chunk",
        ));
    }
    Ok(chunks)
}

fn validate_input(content: &str) -> Result<()> {
    if content.len() > MAX_MEMORY_ARTIFACT_BYTES {
        return Err(MemoryError::too_large(
            "memoryChunkInput",
            MAX_MEMORY_ARTIFACT_BYTES,
        ));
    }
    if content.contains('\0') {
        return Err(MemoryError::invalid(
            "memoryChunkInput",
            "must contain no NUL",
        ));
    }
    Ok(())
}

fn push_non_empty_chunk(
    chunks: &mut Vec<MemoryChunk>,
    parent_artifact_id: &MemoryArtifactId,
    content: &str,
) -> Result<()> {
    if content.trim().is_empty() {
        return Ok(());
    }
    if chunks.len() >= MAX_MEMORY_CHUNKS {
        return Err(MemoryError::too_many("memoryChunks", MAX_MEMORY_CHUNKS));
    }

    let index = u32::try_from(chunks.len())
        .expect("MAX_MEMORY_CHUNKS is representable as a u32 chunk index");
    chunks.push(MemoryChunk {
        id: derive_chunk_id(parent_artifact_id, index, content)?,
        parent_artifact_id: parent_artifact_id.clone(),
        index,
        content: content.to_owned(),
    });
    Ok(())
}

fn derive_chunk_id(
    parent_artifact_id: &MemoryArtifactId,
    index: u32,
    content: &str,
) -> Result<MemoryArtifactId> {
    let mut digest = Sha256::new();
    digest.update(CHUNK_ID_DOMAIN);
    update_digest_field(&mut digest, parent_artifact_id.as_str().as_bytes());
    digest.update(u64::from(index).to_be_bytes());
    update_digest_field(&mut digest, content.as_bytes());
    let digest: [u8; 32] = digest.finalize().into();
    MemoryArtifactId::parse(hex_encode(&digest))
}

fn update_digest_field(digest: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("bounded memory input length fits u64");
    digest.update(length.to_be_bytes());
    digest.update(value);
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(HEX[(byte >> 4) as usize]));
        output.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact_id(value: &str) -> MemoryArtifactId {
        MemoryArtifactId::parse(value).unwrap()
    }

    #[test]
    fn splits_with_pinned_regex_and_preserves_non_separator_bytes() {
        let parent = artifact_id("summary.parent");
        let separator = ChunkSeparatorRegex::new(r"\n{2,}").unwrap();
        let chunks = split_memory_chunks(
            &parent,
            "\n\nfirst ${literal}\n\n두 번째\n\n\n<script>literal</script>\n\n",
            &separator,
        )
        .unwrap();

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].content(), "first ${literal}");
        assert_eq!(chunks[1].content(), "두 번째");
        assert_eq!(chunks[2].content(), "<script>literal</script>");
        for (index, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.parent_artifact_id(), &parent);
            assert_eq!(chunk.index(), u32::try_from(index).unwrap());
        }
    }

    #[test]
    fn chunk_ids_are_deterministic_and_scoped_by_parent_index_and_content() {
        let separator = ChunkSeparatorRegex::new(r"\|").unwrap();
        let parent = artifact_id("parent-a");
        let first = split_memory_chunks(&parent, "same|same", &separator).unwrap();
        let repeated = split_memory_chunks(&parent, "same|same", &separator).unwrap();
        let other_parent =
            split_memory_chunks(&artifact_id("parent-b"), "same|same", &separator).unwrap();

        assert_eq!(first, repeated);
        assert_ne!(first[0].id(), first[1].id());
        assert_ne!(first[0].id(), other_parent[0].id());
        assert_eq!(
            first[0].id().as_str(),
            "2bf77ad13d5f10674cdb398c246a9a13431d542416f793718a95519904fc388f"
        );
        assert_eq!(first[0].id().as_str().len(), 64);
        assert!(
            first[0]
                .id()
                .as_str()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
    }

    #[test]
    fn serialization_exposes_data_but_no_construction_authority() {
        let chunks = split_memory_chunks(
            &artifact_id("parent"),
            "content",
            &ChunkSeparatorRegex::new("---").unwrap(),
        )
        .unwrap();
        let value = serde_json::to_value(&chunks[0]).unwrap();

        assert_eq!(value["parentArtifactId"], "parent");
        assert_eq!(value["index"], 0);
        assert_eq!(value["content"], "content");
        assert_eq!(value["id"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn boundary_separators_and_whitespace_do_not_create_empty_chunks() {
        let parent = artifact_id("parent");
        let separator = ChunkSeparatorRegex::new(",+").unwrap();

        let chunks = split_memory_chunks(&parent, ",,a,,, \t ,,,b,,", &separator).unwrap();
        assert_eq!(
            chunks.iter().map(MemoryChunk::content).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn rejects_input_when_every_segment_is_empty_or_whitespace() {
        let parent = artifact_id("parent");
        let separator = ChunkSeparatorRegex::new(",+").unwrap();

        for input in ["", " \t\n", ",, \t ,,"] {
            assert!(matches!(
                split_memory_chunks(&parent, input, &separator),
                Err(MemoryError::InvalidField { field, .. }) if field == "memoryChunkInput"
            ));
        }
    }

    #[test]
    fn rejects_nul_and_oversize_input_before_splitting() {
        let parent = artifact_id("parent");
        let separator = ChunkSeparatorRegex::new(",").unwrap();

        assert!(matches!(
            split_memory_chunks(&parent, "left\0right", &separator),
            Err(MemoryError::InvalidField { field, .. }) if field == "memoryChunkInput"
        ));
        assert!(matches!(
            split_memory_chunks(
                &parent,
                &"x".repeat(MAX_MEMORY_ARTIFACT_BYTES + 1),
                &separator,
            ),
            Err(MemoryError::PayloadTooLarge { field, max_bytes })
                if field == "memoryChunkInput" && max_bytes == MAX_MEMORY_ARTIFACT_BYTES
        ));
    }

    #[test]
    fn rejects_zero_width_matches_that_only_exist_for_runtime_input() {
        let separator = ChunkSeparatorRegex::new(r"\b").unwrap();
        assert!(matches!(
            split_memory_chunks(&artifact_id("parent"), "word", &separator),
            Err(MemoryError::Regex(
                "separator produced a zero-width runtime match"
            ))
        ));
    }

    #[test]
    fn enforces_exact_chunk_count_boundary() {
        let parent = artifact_id("parent");
        let separator = ChunkSeparatorRegex::new(r"\|").unwrap();
        let at_limit = (0..MAX_MEMORY_CHUNKS)
            .map(|_| "x")
            .collect::<Vec<_>>()
            .join("|");
        assert_eq!(
            split_memory_chunks(&parent, &at_limit, &separator)
                .unwrap()
                .len(),
            MAX_MEMORY_CHUNKS
        );

        let over_limit = format!("{at_limit}|x");
        assert!(matches!(
            split_memory_chunks(&parent, &over_limit, &separator),
            Err(MemoryError::TooManyItems { field, max })
                if field == "memoryChunks" && max == MAX_MEMORY_CHUNKS
        ));
    }
}

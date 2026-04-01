use crate::config::Config;
use crate::diarize::{AttributionSource, Confidence, SpeakerAttribution};
use crate::voice::{self, VoiceProfileWithEmbedding};
use std::collections::HashMap;

// ──────────────────────────────────────────────────────────────
// Speaker identification via enrolled voice profiles.
//
// Runs AFTER diarization — matches per-speaker averaged embeddings
// from DiarizationResult::speaker_embeddings against enrolled
// profiles in ~/.minutes/voices.db.
//
// This is "Level 3" speaker attribution: voice-enrollment-based
// matching that produces High-confidence attributions.
// ──────────────────────────────────────────────────────────────

/// Result of speaker identification against enrolled voice profiles.
#[derive(Debug, Clone)]
pub struct IdentificationResult {
    /// High-confidence mappings: SPEAKER_X → enrolled name
    pub attributions: Vec<SpeakerAttribution>,
    /// Speaker labels that matched no enrolled profile
    pub unmatched: Vec<String>,
}

/// Match diarization speaker embeddings against all enrolled voice profiles.
///
/// `speaker_embeddings` comes from `DiarizationResult::speaker_embeddings` —
/// a map of "SPEAKER_X" → averaged embedding vector for that speaker.
///
/// Returns attributions for every speaker label that matches an enrolled
/// profile above the configured threshold. Handles conflicts: if two
/// speaker labels both match the same profile, the higher-similarity
/// match wins and the other goes to `unmatched`.
pub fn identify_speakers(
    speaker_embeddings: &HashMap<String, Vec<f32>>,
    config: &Config,
) -> IdentificationResult {
    if speaker_embeddings.is_empty() || !config.voice.enabled {
        return IdentificationResult {
            attributions: vec![],
            unmatched: speaker_embeddings.keys().cloned().collect(),
        };
    }

    let threshold = config.voice.match_threshold;

    // Load all enrolled profiles
    let profiles = match voice::open_db() {
        Ok(conn) => match voice::load_all_with_embeddings(&conn) {
            Ok(profiles) if !profiles.is_empty() => profiles,
            _ => {
                return IdentificationResult {
                    attributions: vec![],
                    unmatched: speaker_embeddings.keys().cloned().collect(),
                }
            }
        },
        Err(_) => {
            return IdentificationResult {
                attributions: vec![],
                unmatched: speaker_embeddings.keys().cloned().collect(),
            }
        }
    };

    // For each speaker label, find the best matching profile and score
    let mut candidates: Vec<(String, String, String, f32)> = Vec::new(); // (label, slug, name, similarity)

    for (label, embedding) in speaker_embeddings {
        let mut best_sim = threshold;
        let mut best_profile: Option<&VoiceProfileWithEmbedding> = None;

        for profile in &profiles {
            let sim = voice::cosine_similarity(embedding, &profile.embedding);
            if sim > best_sim {
                best_sim = sim;
                best_profile = Some(profile);
            }
        }

        if let Some(profile) = best_profile {
            candidates.push((
                label.clone(),
                profile.person_slug.clone(),
                profile.name.clone(),
                best_sim,
            ));
        }
    }

    // Resolve conflicts: one-to-one mapping (each profile assigned at most once)
    // Sort by similarity descending so highest-confidence matches win
    candidates.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

    let mut assigned_slugs: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut assigned_labels: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut attributions = Vec::new();

    for (label, slug, name, similarity) in &candidates {
        if assigned_slugs.contains(slug) || assigned_labels.contains(label) {
            continue;
        }
        assigned_slugs.insert(slug.clone());
        assigned_labels.insert(label.clone());

        tracing::info!(
            speaker = %label,
            name = %name,
            similarity = %format!("{:.3}", similarity),
            "voice enrollment match"
        );

        attributions.push(SpeakerAttribution {
            speaker_label: label.clone(),
            name: name.clone(),
            confidence: Confidence::High,
            source: AttributionSource::Enrollment,
        });
    }

    let unmatched: Vec<String> = speaker_embeddings
        .keys()
        .filter(|label| !assigned_labels.contains(*label))
        .cloned()
        .collect();

    if !unmatched.is_empty() {
        tracing::debug!(
            unmatched = ?unmatched,
            "speakers with no voice enrollment match"
        );
    }

    IdentificationResult {
        attributions,
        unmatched,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn setup_db_with_profiles() -> (rusqlite::Connection, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let conn = voice::open_db_at(tmp.path()).unwrap();
        voice::save_profile(&conn, "alice", "Alice", &[1.0, 0.0, 0.0], "test").unwrap();
        voice::save_profile(&conn, "bob", "Bob", &[0.0, 1.0, 0.0], "test").unwrap();
        (conn, tmp)
    }

    #[test]
    fn identify_matches_enrolled_profiles() {
        let (conn, _tmp) = setup_db_with_profiles();
        let profiles = voice::load_all_with_embeddings(&conn).unwrap();

        // Verify Alice's embedding is close to [0.95, 0.1, 0.0]
        let sim_alice = voice::cosine_similarity(&[0.95, 0.1, 0.0], &profiles[0].embedding);
        assert!(sim_alice > 0.5, "sanity: should be above threshold");

        // Test the matching logic directly against loaded profiles
        let mut best_for_0: Option<(&str, f32)> = None;
        for p in &profiles {
            let sim = voice::cosine_similarity(&[0.95, 0.1, 0.0], &p.embedding);
            if sim > 0.5 && (best_for_0.is_none() || sim > best_for_0.unwrap().1) {
                best_for_0 = Some((&p.name, sim));
            }
        }
        assert_eq!(best_for_0.unwrap().0, "Alice");

        let mut best_for_1: Option<(&str, f32)> = None;
        for p in &profiles {
            let sim = voice::cosine_similarity(&[0.05, 0.95, 0.0], &p.embedding);
            if sim > 0.5 && (best_for_1.is_none() || sim > best_for_1.unwrap().1) {
                best_for_1 = Some((&p.name, sim));
            }
        }
        assert_eq!(best_for_1.unwrap().0, "Bob");
    }

    #[test]
    fn empty_embeddings_returns_no_attributions() {
        let config = Config::default();
        let result = identify_speakers(&HashMap::new(), &config);
        assert!(result.attributions.is_empty());
        assert!(result.unmatched.is_empty());
    }

    #[test]
    fn disabled_voice_returns_all_unmatched() {
        let mut config = Config::default();
        config.voice.enabled = false;

        let mut embeddings = HashMap::new();
        embeddings.insert("SPEAKER_0".to_string(), vec![1.0, 0.0]);

        let result = identify_speakers(&embeddings, &config);
        assert!(result.attributions.is_empty());
        assert_eq!(result.unmatched, vec!["SPEAKER_0"]);
    }
}

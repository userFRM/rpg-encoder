//! Helper functions for sharded hierarchy construction workflow.

use crate::server::RpgServer;
use rpg_core::graph::{EntityKind, normalize_path};

impl RpgServer {
    /// Build batch 0: Domain discovery from representative files across all clusters
    pub(crate) async fn build_batch_0_domain_discovery(
        &self,
        total_clusters: usize,
    ) -> Result<String, String> {
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();
        let session_guard = self.hierarchy_session.read().await;
        let session = session_guard.as_ref().unwrap();

        let repo_name = self
            .project_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        // Collect representative files from all clusters
        let mut representative_features = String::new();
        for cluster in &session.clusters {
            for file in &cluster.representatives {
                // Find Module entity for this file
                for entity in graph.entities.values() {
                    if entity.kind == EntityKind::Module
                        && normalize_path(&entity.file) == *file
                        && !entity.semantic_features.is_empty()
                    {
                        representative_features.push_str(&format!(
                            "- {} ({}): {}\n",
                            entity.name,
                            entity.file.display(),
                            entity.semantic_features.join(", ")
                        ));
                        break;
                    }
                }
            }
        }

        let domain_prompt =
            include_str!("../../../crates/rpg-encoder/src/prompts/domain_discovery.md");

        let mut output = String::new();
        output.push_str(&format!(
            "## Semantic Hierarchy Construction for '{}'\n\n",
            repo_name
        ));
        output.push_str(&format!(
            "BATCH 0/{}: Domain Discovery\n\n",
            total_clusters + 1
        ));
        output.push_str(domain_prompt);

        // Inject paradigm-specific discovery hints
        let discovery_hints =
            Self::collect_paradigm_hints(&graph.metadata.paradigms, |h| &h.discovery);
        if !discovery_hints.is_empty() {
            output.push_str("\n\n## Framework-Specific Discovery Guidelines\n\n");
            output.push_str(&discovery_hints);
        }

        output.push_str("\n\n### Representative Files (from clusters):\n");
        output.push_str(&representative_features);

        output.push_str("\n\n## Next Step\n\n");
        output.push_str(
            "Identify 4-8 functional areas that capture the repository's architecture.\n",
        );
        output.push_str("Provide them as JSON:\n```json\n");
        output.push_str("{\"areas\": [\"Area1\", \"Area2\", \"Area3\", ...]}\n");
        output.push_str("```\n\n");
        output.push_str("Then call submit_hierarchy with this JSON, followed by build_semantic_hierarchy for batch 1.");

        Ok(output)
    }

    /// Build file assignment batch for a specific cluster
    pub(crate) async fn build_cluster_batch(
        &self,
        batch_num: usize,
        total_batches: usize,
        cluster: &rpg_encoder::hierarchy::FileCluster,
        functional_areas: &[String],
    ) -> Result<String, String> {
        let guard = self.graph.read().await;
        let graph = guard.as_ref().unwrap();

        let repo_name = self
            .project_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        // Collect file features for this cluster
        let mut file_features = String::new();
        for file in &cluster.files {
            // Find Module entity for this file
            for entity in graph.entities.values() {
                if entity.kind == EntityKind::Module
                    && normalize_path(&entity.file) == *file
                    && !entity.semantic_features.is_empty()
                {
                    file_features.push_str(&format!(
                        "- {} ({}): {}\n",
                        entity.name,
                        entity.file.display(),
                        entity.semantic_features.join(", ")
                    ));
                    break;
                }
            }
        }

        let hierarchy_prompt =
            include_str!("../../../crates/rpg-encoder/src/prompts/hierarchy_construction.md");

        let mut output = String::new();
        output.push_str(&format!(
            "## Semantic Hierarchy Construction for '{}'\n\n",
            repo_name
        ));
        output.push_str(&format!(
            "BATCH {}/{}: File Assignment\n\n",
            batch_num, total_batches
        ));

        output.push_str("### Functional Areas (from batch 0):\n");
        for area in functional_areas {
            output.push_str(&format!("- {}\n", area));
        }

        output.push_str("\n\n### Files in this batch:\n");
        output.push_str(&file_features);

        output.push_str("\n\n## Instructions\n\n");
        output.push_str(hierarchy_prompt);

        // Inject paradigm-specific hierarchy hints
        let hierarchy_hints =
            Self::collect_paradigm_hints(&graph.metadata.paradigms, |h| &h.hierarchy);
        if !hierarchy_hints.is_empty() {
            output.push_str("\n\n## Framework-Specific Hierarchy Patterns\n\n");
            output.push_str(&hierarchy_hints);
        }

        output.push_str("\n\n");
        output.push_str(include_str!("prompts/hierarchy_instructions.md"));

        output.push_str("\n\nProvide assignments as JSON:\n```json\n");
        output.push_str("{\"file1.rs\": \"Area/category/subcategory\", \"file2.rs\": \"Area2/cat/subcat\", ...}\n");
        output.push_str("```\n\n");
        output.push_str("Then call submit_hierarchy with this JSON.");

        Ok(output)
    }
}

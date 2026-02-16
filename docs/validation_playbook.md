# Validation Playbook

This repository now supports four reproducibility flows:

1. Generation runtime telemetry (`run_task_test_loop`, `generation_efficiency_report`)
2. Representation quality controls (`seed_ontology_features`, `assess_representation_quality`)
3. Retrieval/localization ablations (`run_representation_ablation`)
4. Third-party blinded evaluation (`export_external_validation_bundle`)

## Third-party blinded reproduction

1. Build/update RPG for the target commit.
2. Run `export_external_validation_bundle(sample_size=..., k=...)`.
3. Share only `public/tasks.blinded.json` and repo snapshot with external evaluators.
4. Keep `private/answer_key.private.json` hidden from evaluators.
5. Evaluators return predictions via `result_template.json`.
6. Score returned predictions against the private answer key and publish:
   - Acc@k
   - MRR
   - File-level Acc@k
   - Cost and latency per backbone/model

## Minimum report for publication

- Commit SHA / tag
- Tool version
- Model/backbone
- Sandbox mode (`local` or `docker`)
- Prompt/completion tokens
- Estimated cost (USD)
- Median and p95 latency
- Failure type histogram
- Ablation table (full RPG vs snippets-only vs no semantic features)

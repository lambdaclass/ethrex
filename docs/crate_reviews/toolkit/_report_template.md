# {{crate_name}} Complexity & Concurrency Review

Date: {{analysis_date}}
Commit: {{commit_hash}}
Target crate: `{{crate_path}}`

## 1. Quantitative Snapshot

| Type | Code | Blank | Doc comments | Comments | Total |
| --- | --- | --- | --- | --- | --- |
| Main | {{main_code}} | {{main_blank}} | {{main_doc_comments}} | {{main_comments}} | {{main_total}} |
| Tests | {{test_code}} | {{test_blank}} | {{test_doc_comments}} | {{test_comments}} | {{test_total}} |
| Total | {{overall_code}} | {{overall_blank}} | {{overall_doc_comments}} | {{overall_comments}} | {{overall_total}} |

- Files analyzed: {{file_count}} Rust sources (note any exclusions)
- Functions: {{function_total}} total with {{complex_function_total}} flagged as complex (see heuristics)
- Longest routine(s): {{notable_long_functions}}
- Async/concurrency signals (crate-wide):
  - `async fn`: {{async_fn_count}}
  - `.await`: {{await_count}}
  - `tokio::spawn`: {{tokio_spawn_count}}
  - `spawn_blocking`: {{spawn_blocking_count}}
  - `Arc<...>`: {{arc_count}}
  - Mutexes: {{mutex_summary}}
  - Atomics: {{atomic_count}}
  - Other noteworthy primitives: {{other_primitives}}

## 2. High-Risk Components
- {{component_hotspot_1}}
- {{component_hotspot_2}}
- {{component_hotspot_3}}

## 3. Concurrency Observations
- {{concurrency_pattern_1}}
- {{concurrency_pattern_2}}
- {{concurrency_pattern_3}}

## 4. Engineering Complexity Score
- **Score: {{complexity_score}} / 5** — {{score_rationale}}

## 5. Recommendations
1. {{recommendation_1}}
2. {{recommendation_2}}
3. {{recommendation_3}}

## 6. Follow-Ups / Tooling Ideas
- {{follow_up_1}}
- {{follow_up_2}}
- {{follow_up_3}}

# TDD gap audit: cave-pii

- **Crate:** `crates/security/cave-pii` (theme: security)
- **Upstream:** microsoft/presidio @ v2.2.0 (Python)
- **Upstream test symbols:** 1380 (across 171 test files)
- **cave test fns:** 11 (7 PII-specific unit tests in `src/engine.rs` + 4 generic proptest smoke invariants)

## Scope reality

cave-pii is a deliberately minimal subset of Presidio. Its entire `src/` surface is:
`engine::{redact, find_emails, looks_like_credit_card, count_by_type}`, the `models`
(`PiiType`, `PiiDetector`, `PiiFinding`, `PiiScanResult`), and a `GET /api/pii/health`
route. The overwhelming majority of Presidio's 1380 test symbols cover behaviors cave
does not implement: ~80 country-specific recognizers (DE/IT/ES/IN/KR/SG/TR/... national
IDs, passports, tax numbers), NLP/NER engines (spaCy, stanza, transformers, GLiNER,
HuggingFace, Azure/OpenAI LLM recognizers), image + DICOM redaction, OCR, the anonymizer
operator framework (encrypt/decrypt/hash/AES), and the HTTP analyzer/anonymizer/registry
servers. Those are `missing-impl` or `scope-cut` and are NOT padded into the table below.

The audit therefore focuses on the handful of upstream behaviors that map onto cave's
actual implemented surface.

## Gap table

| Behavioral unit | Upstream test | cave impl? | cave test? | gap type | suggested cave test name |
|---|---|---|---|---|---|
| Scan-result aggregation (total + high-confidence flag) | `test_analyzer_engine.py::test_when_threshold_is_zero_then_all_results_pass` / `..._threshold_is_more_than_half...` | yes — `models::PiiScanResult{total_findings, has_high_confidence_pii}` | NO | portable-coverage | `test_pii_scan_result_total_and_high_confidence` |
| Multiple findings counted by entity type | `test_recognizer_result.py` / analyzer count assertions | yes — `engine::count_by_type` | partial (1 happy-path) | portable-coverage | `test_count_by_type_empty_and_single` |
| Redact empty / boundary-length value | `operators/test_redact.py::test_given_value_for_redact_then_we_return_empty_value` | yes — `engine::redact` | partial (len 2/4/12 only) | portable-coverage | `test_redact_empty_and_exact_boundary` |
| Mask preserves prefix/suffix, masks middle | `operators/test_mask.py::test_when_given_valid_value_then_expected_string_returned` | yes — `engine::redact` (keep=2 prefix/suffix) | partial | portable-coverage | `test_redact_preserves_prefix_suffix_chars` |
| Email at end / various positions of line | `test_email_recognizer.py::test_when_all_email_addresses_then_succeed` | yes — `engine::find_emails` | partial (one mid-line case) | portable-coverage | `test_find_emails_at_line_end_and_multiline` |
| PiiType serde round-trip (snake_case wire format) | n/a upstream (cave-specific model) — analog: `test_recognizer_result.py` to_dict | yes — `models::PiiType` `#[serde(rename_all=snake_case)]` | NO | portable-coverage | `test_pii_type_serde_snake_case` |
| Health endpoint contract | `test_api_*` server health checks | yes — `routes::health` | NO | portable-coverage | `test_health_route_reports_module_and_upstream` |
| Credit-card **Luhn checksum** validation | `test_credit_card_recognizer.py::test_when_all_credit_cards_then_succeed` (valid Luhn numbers + invalid checksum rejection) | NO — `looks_like_credit_card` is length-only (16 digits), no checksum | n/a | missing-impl | (needs Luhn impl first) |
| IP address recognition (v4/v6/CIDR span) | `test_ip_recognizer.py` (11 tests) | NO — `PiiType::IpAddress` enum variant only, no detector | n/a | missing-impl | (needs IP detector first) |
| Phone-number recognition | `test_phone_recognizer.py`, country phone tests | NO — `PiiType::PhoneNumber` variant only, no detector | n/a | missing-impl | (needs phone detector first) |
| Anonymizer encrypt/decrypt (AES) | `services/test_aes_cypher.py`, `test_api_anonymizer.py` decrypt tests | NO | n/a | scope-cut | crypto anonymizer operator framework not in cave launch scope |
| Country-specific recognizers (DE/IT/ES/IN/KR/SG/TR ID, passport, tax, IBAN, crypto) | `test_*_recognizer.py` (~80 files) | NO | n/a | scope-cut | per-jurisdiction recognizers out of launch scope |
| NLP/NER engines (spaCy, stanza, transformers, GLiNER, LLM) | `test_spacy_recognizer.py`, `test_transformers_recognizer.py`, etc. | NO | n/a | scope-cut | ML/NLP backends out of launch scope (regex-only crate) |
| Image / DICOM redaction + OCR | `presidio-image-redactor/tests/**` (~200 symbols) | NO | n/a | scope-cut | image pipeline out of launch scope |
| Analyzer/registry HTTP server (ad-hoc recognizers, allow/deny lists, decision process) | `e2e-tests/**`, `test_analyzer_engine.py` (43), `test_recognizer_registry*.py` | NO | n/a | scope-cut | full analyzer server framework out of launch scope |

## Recommended TDD fills (portable-coverage first)

These exercise behavior cave **already implements** but does not test. They are cheap and
verifiable today (no new src/ code required):

1. **`test_pii_scan_result_total_and_high_confidence`** — construct a `models::PiiScanResult`
   with a vec of `PiiFinding`s (mixed confidence) and assert `total_findings` matches the
   vec length and `has_high_confidence_pii` reflects whether any finding crosses the
   high-confidence bar. Exercises `models::PiiScanResult`. (Currently the struct has zero
   coverage.)
2. **`test_count_by_type_empty_and_single`** — call `engine::count_by_type` with an empty
   slice (expect empty map) and a single finding (expect `{type: 1}`). Complements the one
   existing multi-type happy-path test of `engine::count_by_type`.
3. **`test_redact_empty_and_exact_boundary`** — `engine::redact("")` → `""`,
   `redact("abcd")` (len 4, the `<=4` branch boundary) → `"****"`, and `redact("abcde")`
   (len 5, first long case) keeps first/last char and masks the middle. Pins the
   short/long branch boundary of `engine::redact`.
4. **`test_redact_preserves_prefix_suffix_chars`** — assert `engine::redact` keeps exactly
   the first 2 and last 2 chars and that `*` count equals `len - 4`, mirroring Presidio's
   mask operator contract.
5. **`test_find_emails_at_line_end_and_multiline`** — feed `engine::find_emails` a string
   where the email is the last token on a line and a multi-line input with emails on
   several lines; assert correct line numbers and tokens. Extends `find_emails` coverage
   beyond the single mid-line case.
6. **`test_pii_type_serde_snake_case`** — serialize each `models::PiiType` variant and
   assert the snake_case wire form (e.g. `SocialSecurityNumber` → `"social_security_number"`),
   guarding the `#[serde(rename_all = "snake_case")]` contract that the API/JSON layer
   depends on.
7. **`test_health_route_reports_module_and_upstream`** — drive `routes::health` (via the
   axum router) and assert the JSON body reports `module="cave-pii"`, `status="ok"`,
   `upstream="Presidio"`.

### Honest non-fills

- **Luhn checksum** for credit cards is a real `missing-impl`: cave's `looks_like_credit_card`
  only checks for 16 digits, so it would pass `"4111 1111 1111 1112"` (bad checksum) that
  Presidio rejects. Writing a meaningful test here requires implementing Luhn first — out of
  scope for a no-src-change TDD-fill pass, flagged for a follow-up.
- IP and phone recognition are enum variants with no detector — also `missing-impl`, not
  portable.

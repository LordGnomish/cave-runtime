# cave-crm

Sovereign CRM module — function-based reimplementation, Twenty (twentyhq/twenty) upstream

## Status

This module is currently in pre-open-source-launch status. Feature parity with the upstream Twenty application is actively tracked via internal issue boards. Core data models and basic interaction flows are implemented, but advanced automation and reporting features are still under development.

## Upstream

- [twentyhq/twenty](https://github.com/twentyhq/twenty)

## Surface ported

- Contact and company entity models with relational integrity.
- Basic lead and opportunity pipeline management.
- Interaction logging for calls, emails, and meetings.
- Search and filtering capabilities for core entities.
- Role-based access control integration with cave-auth.
- Data export functionality for CSV and JSON formats.
- Basic dashboard metrics for sales performance.
- Webhook support for external system integration.
- Audit logging for all write operations.
- Localization support for primary enterprise languages.

## Public API

- `pub struct CRMContext` provides the main entry point for CRM operations.
- `pub fn create_contact` handles the creation of new contact records.
- `pub fn update_company` manages updates to existing company entities.
- `pub struct LeadPipeline` offers methods for managing sales stages.
- `pub fn search_entities` enables full-text search across CRM objects.
- `pub struct InteractionLog` allows recording of customer interactions.

## Tests

Unit tests cover all core business logic and data validation rules. Integration tests verify database interactions and API endpoints against a mock database. Coverage is currently at 85% for critical paths, with ongoing efforts to improve edge case handling.

## License

Apache-2.0

## See also

- [../cave-auth](../cave-auth)
- [../cave-db](../cave-db)
- [../cave-api](../cave-api)

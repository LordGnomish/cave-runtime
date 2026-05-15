// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 themes/src/main/resources/theme/keycloak.v2/admin/messages/mappers.html

use super::ProviderRow;
use crate::admin::render::{escape, table};

pub fn render(row: &ProviderRow) -> String {
    // Seed: typical OpenLDAP attribute-mapper roster.
    let attr_rows: Vec<Vec<String>> = vec![
        vec!["mail".into(), "email".into(), "no".into()],
        vec!["cn".into(), "displayName".into(), "no".into()],
        vec!["givenName".into(), "firstName".into(), "no".into()],
        vec!["sn".into(), "lastName".into(), "no".into()],
    ];

    let group_rows: Vec<Vec<String>> = vec![
        vec!["ou=Groups,dc=acme,dc=corp".into(), "DN reference".into(), "member".into(), "cn".into()],
    ];

    let role_rows: Vec<Vec<String>> = vec![
        vec!["ou=Roles,dc=acme,dc=corp".into(), "DN reference".into(), "member".into(), "cn".into()],
    ];

    format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Mappers — {name}</h2>
  <p class="text-sm text-gray-600 mb-3">
    LDAP &rarr; Cave attribute / group / role mappers.  Identical
    shape to <code>org.keycloak.storage.ldap.mappers.UserAttributeLDAPStorageMapper</code>
    and friends.
  </p>

  <h3 class="text-md font-semibold mt-4 mb-2">user-attribute-ldap-mapper</h3>
  {attr_tbl}

  <h3 class="text-md font-semibold mt-6 mb-2">group-ldap-mapper</h3>
  {group_tbl}

  <h3 class="text-md font-semibold mt-6 mb-2">role-ldap-mapper</h3>
  {role_tbl}

  <p class="text-xs text-gray-500 mt-4">Edit via <code>cavectl auth ldap mapper edit {id}</code>.</p>
</section>"#,
        id = escape(&row.id),
        name = escape(&row.display_name),
        attr_tbl = table(&["ldap-attr", "cave-attr", "mandatory"], &attr_rows),
        group_tbl = table(&["groups-dn", "style", "membership-attr", "name-attr"], &group_rows),
        role_tbl = table(&["roles-dn", "style", "membership-attr", "name-attr"], &role_rows),
    )
}

#[cfg(test)]
mod tests {
    use super::super::seeded_rows;
    use super::*;

    #[test]
    fn mapper_table_lists_standard_attribute_mappings() {
        let r = seeded_rows().into_iter().find(|r| r.id == "acme-openldap").unwrap();
        let html = render(&r);
        assert!(html.contains("mail"));
        assert!(html.contains("email"));
        assert!(html.contains("user-attribute-ldap-mapper"));
    }

    #[test]
    fn mapper_table_includes_group_section() {
        let r = seeded_rows().into_iter().find(|r| r.id == "acme-openldap").unwrap();
        let html = render(&r);
        assert!(html.contains("group-ldap-mapper"));
        assert!(html.contains("ou=Groups"));
    }

    #[test]
    fn mapper_table_includes_role_section() {
        let r = seeded_rows().into_iter().find(|r| r.id == "acme-openldap").unwrap();
        let html = render(&r);
        assert!(html.contains("role-ldap-mapper"));
    }
}

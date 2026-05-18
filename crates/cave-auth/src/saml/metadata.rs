// SPDX-License-Identifier: AGPL-3.0-or-later
//! `<md:EntityDescriptor>` — the metadata XML two SAML parties
//! exchange before any flow can run. cave-auth emits both SP and
//! IdP descriptors; the SP variant is what a downstream IdP
//! imports to trust cave, and the IdP variant is what SPs import
//! to trust the cave-side IdP.
//!
//! Mirrors `org.keycloak.protocol.saml.SamlService.getDescriptor`
//! (IdP-side) and `org.keycloak.broker.saml.SAMLEndpoint.getSPDescriptor`
//! (SP-side) from upstream.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

use super::{ns, SamlError};

/// Which SAML role this descriptor advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityRole {
    /// Identity Provider — issues Assertions.
    Idp,
    /// Service Provider — consumes Assertions.
    Sp,
}

/// One SingleSignOnService / AssertionConsumerService endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEndpoint {
    /// Spec binding URN (HTTP-Redirect / HTTP-POST).
    pub binding: String,
    /// HTTP URL where the endpoint listens.
    pub location: String,
    /// `index` attribute — required for `AssertionConsumerService`.
    pub index: Option<u32>,
}

/// `<md:EntityDescriptor>` — the top-level metadata document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityDescriptor {
    /// `entityID` — the SAML name for this party.
    pub entity_id: String,
    pub role: EntityRole,
    /// PEM-encoded X.509 cert (or DER-base64 stripped of PEM
    /// markers — both forms are valid and Keycloak emits the
    /// stripped form). Optional only when transport binds
    /// signing externally (rare).
    pub signing_cert_b64: Option<String>,
    /// Endpoints advertised. For an IdP, these are SSO
    /// endpoints; for an SP, ACS endpoints.
    pub endpoints: Vec<ServiceEndpoint>,
    /// Optional SingleLogoutService endpoint (both roles).
    pub slo_endpoint: Option<ServiceEndpoint>,
}

impl EntityDescriptor {
    pub fn new_idp(entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            role: EntityRole::Idp,
            signing_cert_b64: None,
            endpoints: Vec::new(),
            slo_endpoint: None,
        }
    }

    pub fn new_sp(entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            role: EntityRole::Sp,
            signing_cert_b64: None,
            endpoints: Vec::new(),
            slo_endpoint: None,
        }
    }

    pub fn with_signing_cert(mut self, b64_der: impl Into<String>) -> Self {
        self.signing_cert_b64 = Some(b64_der.into());
        self
    }

    pub fn add_endpoint(mut self, binding: impl Into<String>, location: impl Into<String>) -> Self {
        self.endpoints.push(ServiceEndpoint {
            binding: binding.into(),
            location: location.into(),
            index: None,
        });
        self
    }

    pub fn with_slo(mut self, binding: impl Into<String>, location: impl Into<String>) -> Self {
        self.slo_endpoint = Some(ServiceEndpoint {
            binding: binding.into(),
            location: location.into(),
            index: None,
        });
        self
    }

    /// Serialize to a SAML metadata XML document.
    pub fn to_xml(&self) -> Result<Vec<u8>, SamlError> {
        let mut buf = Cursor::new(Vec::new());
        let mut w = Writer::new(&mut buf);

        let mut root = BytesStart::new("md:EntityDescriptor");
        root.push_attribute(("xmlns:md", ns::SAML_METADATA));
        root.push_attribute(("xmlns:ds", ns::XML_DSIG));
        root.push_attribute(("entityID", self.entity_id.as_str()));
        w.write_event(Event::Start(root)).map_err(io_err)?;

        let inner_tag = match self.role {
            EntityRole::Idp => "md:IDPSSODescriptor",
            EntityRole::Sp => "md:SPSSODescriptor",
        };

        let mut inner = BytesStart::new(inner_tag);
        inner.push_attribute(("protocolSupportEnumeration", ns::SAML_PROTOCOL));
        w.write_event(Event::Start(inner)).map_err(io_err)?;

        if let Some(cert) = &self.signing_cert_b64 {
            let mut kd = BytesStart::new("md:KeyDescriptor");
            kd.push_attribute(("use", "signing"));
            w.write_event(Event::Start(kd)).map_err(io_err)?;
            w.write_event(Event::Start(BytesStart::new("ds:KeyInfo"))).map_err(io_err)?;
            w.write_event(Event::Start(BytesStart::new("ds:X509Data"))).map_err(io_err)?;
            w.write_event(Event::Start(BytesStart::new("ds:X509Certificate")))
                .map_err(io_err)?;
            w.write_event(Event::Text(BytesText::new(cert))).map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("ds:X509Certificate"))).map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("ds:X509Data"))).map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("ds:KeyInfo"))).map_err(io_err)?;
            w.write_event(Event::End(BytesEnd::new("md:KeyDescriptor"))).map_err(io_err)?;
        }

        if let Some(slo) = &self.slo_endpoint {
            let mut slo_e = BytesStart::new("md:SingleLogoutService");
            slo_e.push_attribute(("Binding", slo.binding.as_str()));
            slo_e.push_attribute(("Location", slo.location.as_str()));
            w.write_event(Event::Empty(slo_e)).map_err(io_err)?;
        }

        let endpoint_tag = match self.role {
            EntityRole::Idp => "md:SingleSignOnService",
            EntityRole::Sp => "md:AssertionConsumerService",
        };

        for (i, ep) in self.endpoints.iter().enumerate() {
            let mut e = BytesStart::new(endpoint_tag);
            e.push_attribute(("Binding", ep.binding.as_str()));
            e.push_attribute(("Location", ep.location.as_str()));
            if self.role == EntityRole::Sp {
                let idx = ep.index.unwrap_or(i as u32).to_string();
                e.push_attribute(("index", idx.as_str()));
                if i == 0 {
                    e.push_attribute(("isDefault", "true"));
                }
            }
            w.write_event(Event::Empty(e)).map_err(io_err)?;
        }

        w.write_event(Event::End(BytesEnd::new(inner_tag))).map_err(io_err)?;
        w.write_event(Event::End(BytesEnd::new("md:EntityDescriptor"))).map_err(io_err)?;

        Ok(buf.into_inner())
    }

    /// Parse a metadata document. Tolerant of namespace-prefix
    /// variation (Keycloak emits `md:`, some IdPs emit
    /// `xmlns:md` only on the root and unprefix everything else).
    pub fn from_xml(bytes: &[u8]) -> Result<Self, SamlError> {
        let mut reader = Reader::from_reader(bytes);
        reader.config_mut().trim_text(true);

        let mut entity_id = None;
        let mut role = None;
        let mut endpoints = Vec::new();
        let mut slo_endpoint = None;
        let mut signing_cert_b64: Option<String> = None;

        let mut in_key_descriptor = false;
        let mut key_descriptor_use: Option<String> = None;
        let mut in_x509_cert = false;

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Err(e) => return Err(SamlError::Parse(format!("xml: {e}"))),
                Ok(Event::Eof) => break,
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    let name = local_name(e.name().as_ref());
                    match name.as_str() {
                        "EntityDescriptor" => {
                            for a in e.attributes().flatten() {
                                if local_name(a.key.as_ref()) == "entityID" {
                                    let v = a
                                        .unescape_value()
                                        .map_err(|err| SamlError::Parse(err.to_string()))?
                                        .into_owned();
                                    entity_id = Some(v);
                                }
                            }
                        }
                        "IDPSSODescriptor" => role = Some(EntityRole::Idp),
                        "SPSSODescriptor" => role = Some(EntityRole::Sp),
                        "KeyDescriptor" => {
                            in_key_descriptor = true;
                            key_descriptor_use = None;
                            for a in e.attributes().flatten() {
                                if local_name(a.key.as_ref()) == "use" {
                                    let v = a
                                        .unescape_value()
                                        .map_err(|err| SamlError::Parse(err.to_string()))?
                                        .into_owned();
                                    key_descriptor_use = Some(v);
                                }
                            }
                        }
                        "X509Certificate" if in_key_descriptor => in_x509_cert = true,
                        "SingleSignOnService" | "AssertionConsumerService" => {
                            let mut binding = None;
                            let mut location = None;
                            let mut index = None;
                            for a in e.attributes().flatten() {
                                let key = local_name(a.key.as_ref());
                                let val = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?
                                    .into_owned();
                                match key.as_str() {
                                    "Binding" => binding = Some(val),
                                    "Location" => location = Some(val),
                                    "index" => index = val.parse().ok(),
                                    _ => {}
                                }
                            }
                            if let (Some(b), Some(l)) = (binding, location) {
                                endpoints.push(ServiceEndpoint {
                                    binding: b,
                                    location: l,
                                    index,
                                });
                            }
                        }
                        "SingleLogoutService" => {
                            let mut binding = None;
                            let mut location = None;
                            for a in e.attributes().flatten() {
                                let key = local_name(a.key.as_ref());
                                let val = a
                                    .unescape_value()
                                    .map_err(|err| SamlError::Parse(err.to_string()))?
                                    .into_owned();
                                match key.as_str() {
                                    "Binding" => binding = Some(val),
                                    "Location" => location = Some(val),
                                    _ => {}
                                }
                            }
                            if let (Some(b), Some(l)) = (binding, location) {
                                slo_endpoint = Some(ServiceEndpoint {
                                    binding: b,
                                    location: l,
                                    index: None,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(t)) if in_x509_cert => {
                    let v = t
                        .unescape()
                        .map_err(|err| SamlError::Parse(err.to_string()))?
                        .into_owned();
                    if key_descriptor_use.as_deref() != Some("encryption") {
                        signing_cert_b64 = Some(v.trim().to_string());
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name = local_name(e.name().as_ref());
                    match name.as_str() {
                        "KeyDescriptor" => in_key_descriptor = false,
                        "X509Certificate" => in_x509_cert = false,
                        _ => {}
                    }
                }
                _ => {}
            }
            buf.clear();
        }

        Ok(Self {
            entity_id: entity_id.ok_or_else(|| SamlError::MissingField("entityID".into()))?,
            role: role.ok_or_else(|| {
                SamlError::MissingField("IDPSSODescriptor or SPSSODescriptor".into())
            })?,
            signing_cert_b64,
            endpoints,
            slo_endpoint,
        })
    }
}

fn local_name(name: &[u8]) -> String {
    let s = std::str::from_utf8(name).unwrap_or("");
    match s.rfind(':') {
        Some(i) => s[i + 1..].to_string(),
        None => s.to_string(),
    }
}

fn io_err(e: std::io::Error) -> SamlError {
    SamlError::Parse(format!("xml write: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idp_metadata_round_trips() {
        let m = EntityDescriptor::new_idp("https://idp.example")
            .with_signing_cert("BASE64CERTBYTES==")
            .add_endpoint(
                "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect",
                "https://idp.example/sso/redirect",
            )
            .add_endpoint(
                "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST",
                "https://idp.example/sso/post",
            );
        let bytes = m.to_xml().unwrap();
        let s = String::from_utf8(bytes.clone()).unwrap();
        assert!(s.contains("md:IDPSSODescriptor"));
        assert!(s.contains("md:SingleSignOnService"));
        assert!(s.contains("BASE64CERTBYTES=="));

        let parsed = EntityDescriptor::from_xml(&bytes).unwrap();
        assert_eq!(parsed.entity_id, "https://idp.example");
        assert_eq!(parsed.role, EntityRole::Idp);
        assert_eq!(parsed.endpoints.len(), 2);
        assert_eq!(parsed.signing_cert_b64.as_deref(), Some("BASE64CERTBYTES=="));
    }

    #[test]
    fn sp_metadata_emits_index_and_default() {
        let m = EntityDescriptor::new_sp("https://sp.example")
            .add_endpoint(
                "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST",
                "https://sp.example/acs",
            );
        let bytes = m.to_xml().unwrap();
        let s = String::from_utf8(bytes.clone()).unwrap();
        assert!(s.contains("md:SPSSODescriptor"));
        assert!(s.contains("md:AssertionConsumerService"));
        assert!(s.contains("index=\"0\""));
        assert!(s.contains("isDefault=\"true\""));
        let parsed = EntityDescriptor::from_xml(&bytes).unwrap();
        assert_eq!(parsed.role, EntityRole::Sp);
        assert_eq!(parsed.endpoints[0].index, Some(0));
    }

    #[test]
    fn slo_endpoint_round_trips() {
        let m = EntityDescriptor::new_idp("https://idp")
            .with_slo("urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect", "https://idp/slo");
        let bytes = m.to_xml().unwrap();
        let parsed = EntityDescriptor::from_xml(&bytes).unwrap();
        assert_eq!(parsed.slo_endpoint.unwrap().location, "https://idp/slo");
    }

    #[test]
    fn parser_rejects_missing_entity_id() {
        let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata">
            <md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol"/>
        </md:EntityDescriptor>"#;
        assert!(matches!(
            EntityDescriptor::from_xml(xml.as_bytes()).unwrap_err(),
            SamlError::MissingField(_)
        ));
    }
}

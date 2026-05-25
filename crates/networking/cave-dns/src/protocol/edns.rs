// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use hickory_proto::op::Message;
use std::net::IpAddr;

/// Decoded EDNS0 options extracted from the OPT pseudo-record.
#[derive(Debug, Clone, Default)]
pub struct EdnsOptions {
    /// Advertised UDP payload size.
    pub udp_payload_size: u16,
    /// DO (DNSSEC OK) bit.
    pub dnssec_ok: bool,
    /// NSID option requested.
    pub nsid: bool,
    /// EDNS Client Subnet option.
    pub client_subnet: Option<ClientSubnet>,
}

/// RFC 7871 EDNS Client Subnet.
#[derive(Debug, Clone)]
pub struct ClientSubnet {
    pub source_prefix_len: u8,
    pub scope_prefix_len: u8,
    pub address: IpAddr,
}

impl EdnsOptions {
    /// Extract EDNS options from a DNS request message.
    pub fn from_message(msg: &Message) -> Self {
        match msg.extensions() {
            None => Self::default(),
            Some(edns) => Self {
                udp_payload_size: edns.max_payload(),
                dnssec_ok: edns.dnssec_ok(),
                nsid: false,         // would parse OPT options in depth
                client_subnet: None, // would parse option code 8
            },
        }
    }

    /// Effective UDP buffer size: at least 512 bytes.
    pub fn effective_udp_size(&self) -> u16 {
        self.udp_payload_size.max(512)
    }
}

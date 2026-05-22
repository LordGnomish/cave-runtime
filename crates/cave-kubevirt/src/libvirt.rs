// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! libvirt Domain XML emitter.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   pkg/virt-launcher/virtwrap/converter/converter.go (Converter.Convert)
//!   pkg/virt-launcher/virtwrap/api/schema.go (Domain XML schema)
//!
//! This module owns the canonical translation from a `VirtualMachineInstance`
//! spec to the libvirt domain XML wire format that the qemu emulator
//! understands. Output is deterministic — same input always renders the same
//! XML — so the virt-launcher can hash it for change-detection.
//!
//! The XML emitter is allocation-light: no DOM is built, the rendering is a
//! direct write to a `String`. This mirrors the upstream `xml.MarshalIndent`
//! path but lets us avoid pulling in an XML library for what is effectively a
//! small fixed schema.

use crate::models::{
    Domain, DomainCpu, DomainMemory, Firmware, Network, VirtualMachineInstance, Volume,
};
use std::fmt::Write as _;

/// The libvirt domain type emitted. KubeVirt always uses `kvm`.
pub const DOMAIN_TYPE: &str = "kvm";

/// Default machine type — `pc-q35` on x86_64, matching upstream.
pub const DEFAULT_MACHINE_TYPE: &str = "pc-q35-rhel8.6.0";

/// Architecture — KubeVirt today is x86_64-only in mainline (`aarch64`
/// behind a feature gate). We expose both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    X86_64,
    Aarch64,
}

impl Architecture {
    pub fn libvirt_arch(&self) -> &'static str {
        match self {
            Architecture::X86_64 => "x86_64",
            Architecture::Aarch64 => "aarch64",
        }
    }

    pub fn default_emulator(&self) -> &'static str {
        match self {
            Architecture::X86_64 => "/usr/libexec/qemu-kvm",
            Architecture::Aarch64 => "/usr/libexec/qemu-aarch64",
        }
    }
}

/// Options that control the rendering. Defaults match KubeVirt's
/// virt-launcher defaults so the simple path needs no configuration.
#[derive(Debug, Clone)]
pub struct EmitOptions {
    pub architecture: Architecture,
    pub machine_type: String,
    pub uuid: String,
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self {
            architecture: Architecture::X86_64,
            machine_type: DEFAULT_MACHINE_TYPE.into(),
            uuid: String::new(),
        }
    }
}

/// Emit the libvirt domain XML for a VirtualMachineInstance. The result
/// is suitable for `virsh define -` or `virConnectDomainXMLFromNative`.
pub fn emit_domain_xml(vmi: &VirtualMachineInstance, opts: &EmitOptions) -> String {
    let mut out = String::with_capacity(2048);
    let name = format!(
        "{}_{}",
        vmi.namespace.as_deref().unwrap_or("default"),
        vmi.name
    );
    let uuid = if opts.uuid.is_empty() {
        format!("00000000-0000-0000-0000-{:012x}", hash_name(&name))
    } else {
        opts.uuid.clone()
    };

    let _ = writeln!(out, "<domain type='{}'>", DOMAIN_TYPE);
    let _ = writeln!(out, "  <name>{}</name>", xml_escape(&name));
    let _ = writeln!(out, "  <uuid>{}</uuid>", xml_escape(&uuid));
    emit_memory(&mut out, &vmi.spec.domain.memory);
    emit_cpu(&mut out, &vmi.spec.domain.cpu);
    emit_os(&mut out, opts, &vmi.spec.domain.firmware);
    emit_features(&mut out);
    emit_clock(&mut out, opts.architecture);
    emit_devices(&mut out, opts.architecture, &vmi.spec.volumes, &vmi.spec.networks);
    let _ = writeln!(out, "</domain>");
    out
}

fn emit_memory(out: &mut String, mem: &Option<DomainMemory>) {
    let guest = mem
        .as_ref()
        .and_then(|m| m.guest.as_ref())
        .map(String::as_str)
        .unwrap_or("1024Mi");
    let bytes = parse_quantity(guest).unwrap_or(1024 * 1024 * 1024);
    let kib = bytes / 1024;
    let _ = writeln!(out, "  <memory unit='KiB'>{}</memory>", kib);
    let _ = writeln!(out, "  <currentMemory unit='KiB'>{}</currentMemory>", kib);
    if let Some(huge) = mem.as_ref().and_then(|m| m.hugepages.as_ref()) {
        let _ = writeln!(out, "  <memoryBacking>");
        let _ = writeln!(out, "    <hugepages>");
        let _ = writeln!(
            out,
            "      <page size='{}' unit='b'/>",
            parse_quantity(&huge.page_size).unwrap_or(2 * 1024 * 1024)
        );
        let _ = writeln!(out, "    </hugepages>");
        let _ = writeln!(out, "  </memoryBacking>");
    }
}

fn emit_cpu(out: &mut String, cpu: &Option<DomainCpu>) {
    let (cores, sockets, threads) = cpu
        .as_ref()
        .map(|c| (c.cores.unwrap_or(1), c.sockets.unwrap_or(1), c.threads.unwrap_or(1)))
        .unwrap_or((1, 1, 1));
    let vcpus = cores as u64 * sockets as u64 * threads as u64;
    let _ = writeln!(out, "  <vcpu placement='static'>{}</vcpu>", vcpus);
    let model = cpu
        .as_ref()
        .and_then(|c| c.model.as_deref())
        .unwrap_or("host-passthrough");
    if model == "host-passthrough" {
        let _ = writeln!(
            out,
            "  <cpu mode='host-passthrough'>\
             \n    <topology sockets='{}' cores='{}' threads='{}'/>\
             \n  </cpu>",
            sockets, cores, threads
        );
    } else {
        let _ = writeln!(
            out,
            "  <cpu mode='custom' match='exact'>\
             \n    <model>{}</model>\
             \n    <topology sockets='{}' cores='{}' threads='{}'/>\
             \n  </cpu>",
            xml_escape(model),
            sockets,
            cores,
            threads
        );
    }
}

fn emit_os(out: &mut String, opts: &EmitOptions, firmware: &Option<Firmware>) {
    let _ = writeln!(out, "  <os>");
    let _ = writeln!(
        out,
        "    <type arch='{}' machine='{}'>hvm</type>",
        opts.architecture.libvirt_arch(),
        xml_escape(&opts.machine_type)
    );
    if let Some(fw) = firmware {
        if let Some(bootloader) = &fw.bootloader {
            if bootloader.eq_ignore_ascii_case("efi") {
                let _ = writeln!(
                    out,
                    "    <loader readonly='yes' type='pflash'>/usr/share/OVMF/OVMF_CODE.fd</loader>"
                );
            }
        }
    }
    let _ = writeln!(out, "    <boot dev='hd'/>");
    let _ = writeln!(out, "  </os>");
}

fn emit_features(out: &mut String) {
    let _ = writeln!(out, "  <features>");
    let _ = writeln!(out, "    <acpi/>");
    let _ = writeln!(out, "    <apic/>");
    let _ = writeln!(out, "  </features>");
}

fn emit_clock(out: &mut String, arch: Architecture) {
    let offset = match arch {
        Architecture::X86_64 => "utc",
        Architecture::Aarch64 => "utc",
    };
    let _ = writeln!(out, "  <clock offset='{}'>", offset);
    if arch == Architecture::X86_64 {
        let _ = writeln!(out, "    <timer name='rtc' tickpolicy='catchup'/>");
        let _ = writeln!(out, "    <timer name='pit' tickpolicy='delay'/>");
        let _ = writeln!(out, "    <timer name='hpet' present='no'/>");
    }
    let _ = writeln!(out, "  </clock>");
}

fn emit_devices(out: &mut String, arch: Architecture, volumes: &[Volume], networks: &[Network]) {
    let _ = writeln!(out, "  <devices>");
    let _ = writeln!(
        out,
        "    <emulator>{}</emulator>",
        arch.default_emulator()
    );
    for (i, v) in volumes.iter().enumerate() {
        emit_disk(out, i, v);
    }
    for (i, n) in networks.iter().enumerate() {
        emit_interface(out, i, n);
    }
    // Console + VNC are universal.
    let _ = writeln!(
        out,
        "    <serial type='pty'>\n      <target type='isa-serial' port='0'/>\n    </serial>"
    );
    let _ = writeln!(out, "    <console type='pty'/>");
    let _ = writeln!(out, "    <graphics type='vnc' autoport='yes'/>");
    let _ = writeln!(out, "  </devices>");
}

fn emit_disk(out: &mut String, idx: usize, v: &Volume) {
    let kind = v
        .source
        .get("kind")
        .and_then(|k| k.as_str())
        .unwrap_or("PersistentVolumeClaim");
    let pvc_name = v
        .source
        .get("name")
        .and_then(|k| k.as_str())
        .unwrap_or(&v.name);
    let dev = format!("vd{}", char::from(b'a' + (idx % 26) as u8));
    let _ = writeln!(out, "    <disk type='block' device='disk'>");
    let _ = writeln!(out, "      <driver name='qemu' type='raw'/>");
    let _ = writeln!(out, "      <target dev='{}' bus='virtio'/>", dev);
    let alias = format!("ua-{}-{}", kind.to_ascii_lowercase(), xml_escape(pvc_name));
    let _ = writeln!(out, "      <alias name='{}'/>", alias);
    let _ = writeln!(out, "      <serial>{}</serial>", xml_escape(&v.name));
    let _ = writeln!(out, "    </disk>");
}

fn emit_interface(out: &mut String, idx: usize, n: &Network) {
    let model = n
        .source
        .get("model")
        .and_then(|k| k.as_str())
        .unwrap_or("virtio");
    let mac = format!("52:54:00:{:02x}:{:02x}:{:02x}", idx, idx + 1, idx + 2);
    let _ = writeln!(out, "    <interface type='bridge'>");
    let _ = writeln!(out, "      <source bridge='k6t-eth{}'/>", idx);
    let _ = writeln!(out, "      <mac address='{}'/>", mac);
    let _ = writeln!(out, "      <model type='{}'/>", model);
    let _ = writeln!(out, "      <alias name='ua-{}'/>", xml_escape(&n.name));
    let _ = writeln!(out, "    </interface>");
}

/// Round-trip an arbitrary `Domain` to the wire format the rest of the
/// node-agent code uses. Currently a stub that captures the input as a
/// JSON-shaped opaque structure — extended in follow-up.
pub fn capture_domain(domain: &Domain) -> serde_json::Value {
    serde_json::json!({
        "cpu": domain.cpu,
        "memory": domain.memory,
        "firmware": domain.firmware,
    })
}

/// Parse a Kubernetes quantity ("1Gi", "512Mi", "1024Ki", "1024") into bytes.
/// Accepts the binary prefixes used by KubeVirt; decimal prefixes are
/// converted similarly.
pub fn parse_quantity(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let suffixes: &[(&str, u64)] = &[
        ("Ei", 1u64 << 60),
        ("Pi", 1u64 << 50),
        ("Ti", 1u64 << 40),
        ("Gi", 1u64 << 30),
        ("Mi", 1u64 << 20),
        ("Ki", 1u64 << 10),
        ("E", 1_000_000_000_000_000_000),
        ("P", 1_000_000_000_000_000),
        ("T", 1_000_000_000_000),
        ("G", 1_000_000_000),
        ("M", 1_000_000),
        ("k", 1_000),
        ("K", 1_000),
    ];
    for (suffix, mult) in suffixes {
        if let Some(rest) = s.strip_suffix(suffix) {
            let n: f64 = rest.trim().parse().ok()?;
            return Some((n * (*mult as f64)) as u64);
        }
    }
    s.parse::<u64>().ok()
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Deterministic 12-hex-digit hash of a name. Used to derive a stable UUID
/// suffix when none is supplied. Implementation: FNV-1a 64-bit truncated.
fn hash_name(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h & 0xFFFFFFFFFFFF
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        DomainCpu, DomainMemory, Firmware, Network, VirtualMachineInstance,
        VirtualMachineInstanceSpec, Volume,
    };

    fn sample_vmi() -> VirtualMachineInstance {
        let mut vmi = VirtualMachineInstance::default();
        vmi.name = "vm-1".into();
        vmi.namespace = Some("default".into());
        vmi.spec = VirtualMachineInstanceSpec {
            domain: Domain {
                cpu: Some(DomainCpu {
                    cores: Some(2),
                    sockets: Some(1),
                    threads: Some(1),
                    model: Some("host-passthrough".into()),
                }),
                memory: Some(DomainMemory {
                    guest: Some("2Gi".into()),
                    hugepages: None,
                }),
                devices: serde_json::json!({}),
                firmware: Some(Firmware {
                    uuid: None,
                    bootloader: Some("efi".into()),
                    serial: None,
                }),
                features: serde_json::json!({}),
            },
            volumes: vec![Volume {
                name: "rootdisk".into(),
                source: serde_json::json!({"kind": "PersistentVolumeClaim", "name": "rootpvc"}),
            }],
            networks: vec![Network {
                name: "default".into(),
                source: serde_json::json!({"model": "virtio"}),
            }],
            termination_grace_period_seconds: Some(30),
            eviction_strategy: None,
        };
        vmi
    }

    #[test]
    fn emit_domain_is_kvm() {
        let xml = emit_domain_xml(&sample_vmi(), &EmitOptions::default());
        assert!(xml.starts_with("<domain type='kvm'>"));
        assert!(xml.contains("<name>default_vm-1</name>"));
        assert!(xml.contains("<vcpu placement='static'>2</vcpu>"));
    }

    #[test]
    fn emit_domain_includes_memory_in_kib() {
        let xml = emit_domain_xml(&sample_vmi(), &EmitOptions::default());
        // 2Gi == 2 * 1024 * 1024 * 1024 bytes; /1024 = 2,097,152 KiB
        assert!(xml.contains("<memory unit='KiB'>2097152</memory>"));
    }

    #[test]
    fn emit_domain_emits_host_passthrough_cpu() {
        let xml = emit_domain_xml(&sample_vmi(), &EmitOptions::default());
        assert!(xml.contains("<cpu mode='host-passthrough'>"));
        assert!(xml.contains("sockets='1' cores='2' threads='1'"));
    }

    #[test]
    fn emit_domain_emits_efi_loader_when_requested() {
        let xml = emit_domain_xml(&sample_vmi(), &EmitOptions::default());
        assert!(xml.contains("OVMF_CODE.fd"));
    }

    #[test]
    fn emit_domain_skips_efi_loader_when_no_firmware() {
        let mut vmi = sample_vmi();
        vmi.spec.domain.firmware = None;
        let xml = emit_domain_xml(&vmi, &EmitOptions::default());
        assert!(!xml.contains("OVMF_CODE.fd"));
    }

    #[test]
    fn emit_domain_renders_disks() {
        let xml = emit_domain_xml(&sample_vmi(), &EmitOptions::default());
        assert!(xml.contains("<disk type='block' device='disk'>"));
        assert!(xml.contains("<target dev='vda' bus='virtio'/>"));
        assert!(xml.contains("<serial>rootdisk</serial>"));
    }

    #[test]
    fn emit_domain_renders_interfaces() {
        let xml = emit_domain_xml(&sample_vmi(), &EmitOptions::default());
        assert!(xml.contains("<interface type='bridge'>"));
        assert!(xml.contains("<source bridge='k6t-eth0'/>"));
        assert!(xml.contains("<model type='virtio'/>"));
    }

    #[test]
    fn emit_domain_is_deterministic() {
        let a = emit_domain_xml(&sample_vmi(), &EmitOptions::default());
        let b = emit_domain_xml(&sample_vmi(), &EmitOptions::default());
        assert_eq!(a, b);
    }

    #[test]
    fn emit_domain_explicit_uuid_used() {
        let opts = EmitOptions {
            uuid: "11111111-2222-3333-4444-555555555555".into(),
            ..Default::default()
        };
        let xml = emit_domain_xml(&sample_vmi(), &opts);
        assert!(xml.contains("<uuid>11111111-2222-3333-4444-555555555555</uuid>"));
    }

    #[test]
    fn emit_domain_emits_hugepages() {
        let mut vmi = sample_vmi();
        vmi.spec.domain.memory = Some(DomainMemory {
            guest: Some("2Gi".into()),
            hugepages: Some(crate::models::HugepagesSpec {
                page_size: "2Mi".into(),
            }),
        });
        let xml = emit_domain_xml(&vmi, &EmitOptions::default());
        assert!(xml.contains("<hugepages>"));
        assert!(xml.contains("<page size='2097152' unit='b'/>"));
    }

    #[test]
    fn emit_domain_emits_aarch64() {
        let opts = EmitOptions {
            architecture: Architecture::Aarch64,
            ..Default::default()
        };
        let xml = emit_domain_xml(&sample_vmi(), &opts);
        assert!(xml.contains("arch='aarch64'"));
        assert!(xml.contains("qemu-aarch64"));
    }

    #[test]
    fn parse_quantity_binary_prefixes() {
        assert_eq!(parse_quantity("1Ki"), Some(1024));
        assert_eq!(parse_quantity("1Mi"), Some(1024 * 1024));
        assert_eq!(parse_quantity("1Gi"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_quantity("2Gi"), Some(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn parse_quantity_decimal_prefixes() {
        assert_eq!(parse_quantity("1k"), Some(1000));
        assert_eq!(parse_quantity("1M"), Some(1_000_000));
        assert_eq!(parse_quantity("1G"), Some(1_000_000_000));
    }

    #[test]
    fn parse_quantity_no_suffix() {
        assert_eq!(parse_quantity("1024"), Some(1024));
        assert_eq!(parse_quantity("42"), Some(42));
    }

    #[test]
    fn parse_quantity_rejects_garbage() {
        assert!(parse_quantity("notanumber").is_none());
        assert!(parse_quantity("").is_none());
    }

    #[test]
    fn xml_escape_handles_special_chars() {
        assert_eq!(xml_escape("a&b"), "a&amp;b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("\"x\""), "&quot;x&quot;");
        assert_eq!(xml_escape("'x'"), "&apos;x&apos;");
    }

    #[test]
    fn hash_name_is_stable() {
        assert_eq!(hash_name("foo"), hash_name("foo"));
        assert_ne!(hash_name("foo"), hash_name("bar"));
    }

    #[test]
    fn architecture_emulator_paths() {
        assert!(Architecture::X86_64.default_emulator().contains("qemu-kvm"));
        assert!(Architecture::Aarch64.default_emulator().contains("aarch64"));
    }

    #[test]
    fn custom_cpu_model() {
        let mut vmi = sample_vmi();
        vmi.spec.domain.cpu = Some(DomainCpu {
            cores: Some(4),
            sockets: Some(2),
            threads: Some(1),
            model: Some("Haswell".into()),
        });
        let xml = emit_domain_xml(&vmi, &EmitOptions::default());
        assert!(xml.contains("<cpu mode='custom' match='exact'>"));
        assert!(xml.contains("<model>Haswell</model>"));
        assert!(xml.contains("<vcpu placement='static'>8</vcpu>"));
    }

    #[test]
    fn capture_domain_round_trip() {
        let d = sample_vmi().spec.domain.clone();
        let v = capture_domain(&d);
        assert!(v.get("cpu").is_some());
        assert!(v.get("memory").is_some());
    }
}

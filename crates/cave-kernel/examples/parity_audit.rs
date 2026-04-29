fn main() {
    for (name, crate_dir) in [
        ("cave-cri", "crates/cave-cri"),
        ("cave-etcd", "crates/cave-etcd"),
        ("cave-apiserver", "crates/cave-apiserver"),
        ("cave-kubelet", "crates/cave-kubelet"),
        ("cave-scheduler", "crates/cave-scheduler"),
        ("cave-controller-manager", "crates/cave-controller-manager"),
        ("cave-cloud-controller-manager", "crates/cave-cloud-controller-manager"),
    ] {
        let manifest_path = format!("{}/parity.manifest.toml", crate_dir);
        let toml_str = std::fs::read_to_string(&manifest_path).expect("read manifest");
        let report = cave_kernel::parity::calculate_from_str(&toml_str, crate_dir).expect("calc");
        println!("\n===== {} =====", name);
        println!("upstream:  {}", report.upstream_ref);
        println!("file:      {} / {} = {:.3}", report.file_parity.matched, report.file_parity.total, report.file_parity.score);
        println!("function:  {} / {} = {:.3}", report.function_parity.matched, report.function_parity.total, report.function_parity.score);
        println!("test:      {} / {} = {:.3}", report.test_parity.matched, report.test_parity.total, report.test_parity.score);
        println!("surface:   {} / {} = {:.3}", report.surface_parity.matched, report.surface_parity.total, report.surface_parity.score);
        println!("overall:   {:.3}", report.overall);
        println!("stubs:     {}", report.stubs_detected);
        println!("gap count: {}", report.gaps.len());
        for g in &report.gaps {
            println!("  GAP {:?}  {} -> {:?}", g.kind, g.upstream, g.local);
        }
    }
}

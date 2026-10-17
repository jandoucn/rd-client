fn main() {
    println!("cargo:rustc-check-cfg=cfg(t02_receipts)");
    println!("cargo:rustc-check-cfg=cfg(t02_integration)");
    println!("cargo:rustc-cfg=t02_receipts");
}

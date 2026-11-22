fn main() {
    println!("cargo:rerun-if-changed=res/app.rc");
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=res/app.ico");
    println!("cargo:rerun-if-changed=installer/Trimly.iss");

    // The version lives in three files. Fail early if they disagree, since a mismatched installer
    // or exe version breaks upgrades and the update check.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let commas = format!("{},0", version.replace('.', ","));
    let rc = std::fs::read_to_string("res/app.rc").unwrap();
    let iss = std::fs::read_to_string("installer/Trimly.iss").unwrap();
    for (file, needle, found) in [
        ("res/app.rc", format!("\"{version}\""), rc.contains(&format!("\"FileVersion\",      \"{version}\""))),
        ("res/app.rc", commas.clone(), rc.contains(&format!("FILEVERSION     {commas}"))),
        ("installer/Trimly.iss", version.clone(), iss.contains(&format!("#define AppVersion \"{version}\""))),
    ] {
        assert!(found, "{file} must use version {needle} to match Cargo.toml");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Embeds the icon, version info and the manifest (per-monitor DPI awareness).
        embed_resource::compile("res/app.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}

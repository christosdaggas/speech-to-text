use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let data_dir = manifest_dir.join("data").join("resources");
    let gresource_xml = data_dir.join("com.chrisdaggas.speech-to-text.gresource.xml");
    let output = out_dir.join("speech-to-text.gresource");

    // Compile GResource bundle (CSS, icons, etc.)
    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir")
        .arg(&data_dir)
        .arg("--target")
        .arg(&output)
        .arg(&gresource_xml)
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:rerun-if-changed=data/resources/com.chrisdaggas.speech-to-text.gresource.xml");
            println!("cargo:rerun-if-changed=data/resources/style.css");
            println!("cargo:rustc-cfg=feature=\"gresource\"");
        }
        _ => {
            eprintln!("Warning: glib-compile-resources not available, using fallback CSS loading");
        }
    }

    // Compile .po → .mo files if msgfmt is available
    let po_dir = manifest_dir.join("po");
    if po_dir.exists() {
        let locales = ["de", "el", "es", "fr", "it", "pt", "ru", "zh"];

        for locale in &locales {
            let po_file = po_dir.join(format!("{}.po", locale));
            if po_file.exists() {
                let mo_dir = manifest_dir
                    .join("data")
                    .join("locale")
                    .join(locale)
                    .join("LC_MESSAGES");
                std::fs::create_dir_all(&mo_dir).ok();
                let mo_file = mo_dir.join("speech-to-text.mo");

                let result = Command::new("msgfmt")
                    .args([
                        po_file.to_str().unwrap(),
                        "-o",
                        mo_file.to_str().unwrap(),
                    ])
                    .status();

                match result {
                    Ok(s) if s.success() => {
                        println!("cargo:warning=Compiled {}.po → {}.mo", locale, locale);
                    }
                    _ => {
                        println!(
                            "cargo:warning=msgfmt failed for {}.po (gettext tools not installed?)",
                            locale
                        );
                    }
                }
            }
            println!("cargo:rerun-if-changed=po/{}.po", locale);
        }
    }
}

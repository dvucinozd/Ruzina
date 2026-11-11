//! Compiles the deck screen for the software renderer, only with `slint-ui`.
fn main() {
    #[cfg(feature = "slint-ui")]
    {
        let ui = "ui/overview.slint";
        println!("cargo:rerun-if-changed={ui}");
        let config = slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer)
            .with_style("fluent".into());
        slint_build::compile_with_config(ui, config).expect("deck screen compiles");
    }
}

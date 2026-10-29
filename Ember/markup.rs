use crate::ASSEMBLY;
pub(crate) use sample_support::xaml_class;
use sample_support::Sample;
pub use sample_support::XamlClass;
include!(concat!(env!("OUT_DIR"), "/assets.rs"));
include!(concat!(env!("OUT_DIR"), "/compiled_classes.rs"));
pub static SAMPLE: Sample = Sample {
    assembly: &ASSEMBLY,
    register_types: crate::register_types,
    classes: &[crate::register_types::CLASSES],
    compiled: COMPILED,
    assets: ASSETS,
    #[cfg(test)]
    document_assets: DOCUMENT_ASSETS,
    #[cfg(not(test))]
    document_assets: &[],
    documents: DOCUMENTS,
};

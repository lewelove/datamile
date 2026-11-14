use libdatamile::lua::{LogicManifest, OrderDef};

use super::{View, default_library, defined_for, ordered_keys};

fn manifest(libraries: &[&str], orders: &[&str]) -> LogicManifest {
    let mut manifest = LogicManifest::default();

    for key in libraries {
        manifest
            .libraries
            .insert((*key).to_string(), libdatamile::lua::LibraryDef::default());
    }
    for key in orders {
        manifest
            .orders
            .insert((*key).to_string(), OrderDef::default());
    }

    manifest
}

/// An explicit preferred library wins over declaration order.
#[test]
fn default_library_prefers_library_key() {
    let manifest = manifest(&["all_albums", "library"], &[]);
    assert_eq!(default_library(&manifest), "library");
}

/// Without a `library` key the first declared library is used.
#[test]
fn default_library_falls_back_to_first() {
    let mut manifest = manifest(&["all_albums"], &[]);
    manifest.libraries_order = vec!["all_albums".to_string()];
    assert_eq!(default_library(&manifest), "all_albums");
}

/// Untyped definitions stay visible; foreign typed ones are hidden.
#[test]
fn definitions_scope_by_type() {
    fn type_of(def: &OrderDef) -> Option<&str> {
        def.r#type.as_deref()
    }

    let mut def = OrderDef::default();
    assert!(defined_for(&def, Some("album"), type_of));

    def.r#type = Some("film".to_string());
    assert!(!defined_for(&def, Some("album"), type_of));
    assert!(defined_for(&def, Some("film"), type_of));
    assert!(defined_for(&def, None, type_of));
}

/// Ordering drops keys that the type scope filtered out.
#[test]
fn order_keeps_declared_sequence() {
    let manifest = manifest(&[], &["az", "year"]);
    let declared = vec!["year".to_string(), "az".to_string()];
    let order = ordered_keys(&manifest.orders, &declared, Some("album"), |def| {
        def.r#type.as_deref()
    });
    assert_eq!(order, vec!["year".to_string(), "az".to_string()]);
}

/// `view=none` yields ids only; comma lists keep their order.
#[test]
fn view_parses_sections() {
    assert!(matches!(View::parse(None), View::Full));
    assert!(matches!(View::parse(Some("none")), View::None));
    assert!(matches!(View::parse(Some("")), View::None));

    let View::Sections(sections) = View::parse(Some("tags, objects,files")) else {
        panic!("expected sections");
    };
    assert_eq!(sections, vec!["tags", "objects", "files"]);
}

#![allow(non_snake_case)]

mod common;

use rust_dicore::*;
use rust_dicore_macros::Inject;
use std::sync::Arc;

#[derive(Inject)]
struct FullFeatured {
    log: Arc<common::Logger>,
    #[inject(optional)]
    maybe: Option<Arc<common::MyService>>,
    #[inject(key = "special")]
    special: Arc<common::Logger>,
    #[inject(skip)]
    label: String,
}

#[test]
fn inject_full_featured() {
    let p = ServiceCollection::new()
        .singleton(|_| {
            Arc::new(common::Logger {
                prefix: "main".into(),
            })
        })
        .keyed("special", |_| {
            Arc::new(common::Logger {
                prefix: "special".into(),
            })
        })
        .build()
        .unwrap();
    let obj = __rdi_construct_FullFeatured(&p);
    assert_eq!(obj.log.prefix, "main");
    assert!(obj.maybe.is_none());
    assert_eq!(obj.special.prefix, "special");
    assert_eq!(obj.label, "");
}

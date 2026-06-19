//! Architecture pattern integration tests.
//! Each pattern is self-contained with its own local types.

use rust_dicore::*;
use std::sync::Arc;

// ── Strategy Pattern ──

trait Strategy: Send + Sync {
    fn execute(&self) -> &'static str;
}

struct FastStrategy;
impl Strategy for FastStrategy {
    fn execute(&self) -> &'static str {
        "fast"
    }
}

struct SlowStrategy;
impl Strategy for SlowStrategy {
    fn execute(&self) -> &'static str {
        "slow"
    }
}

#[test]
fn strategy_pattern() {
    let p = ServiceCollection::new()
        .singleton::<dyn Strategy>(|_| Arc::new(FastStrategy))
        .keyed::<dyn Strategy>("slow", |_| Arc::new(SlowStrategy))
        .build()
        .unwrap();
    let fast: Arc<dyn Strategy> = p.get();
    assert_eq!(fast.execute(), "fast");
    let slow: Arc<dyn Strategy> = p.get_keyed("slow");
    assert_eq!(slow.execute(), "slow");
}

// ── Factory Pattern ──

trait Product: Send + Sync {
    fn kind(&self) -> &'static str;
}

struct ProductA;
impl Product for ProductA {
    fn kind(&self) -> &'static str {
        "A"
    }
}

struct ProductB;
impl Product for ProductB {
    fn kind(&self) -> &'static str {
        "B"
    }
}

#[test]
fn factory_pattern() {
    let p = ServiceCollection::new()
        .keyed::<dyn Product>("a", |_| Arc::new(ProductA))
        .keyed::<dyn Product>("b", |_| Arc::new(ProductB))
        .build()
        .unwrap();
    let a: Arc<dyn Product> = p.get_keyed("a");
    let b: Arc<dyn Product> = p.get_keyed("b");
    assert_eq!(a.kind(), "A");
    assert_eq!(b.kind(), "B");
}

// ── Decorator Pattern ──

trait Notifier: Send + Sync {
    fn send(&self) -> String;
}

struct BaseNotifier;
impl Notifier for BaseNotifier {
    fn send(&self) -> String {
        "base".to_string()
    }
}

struct LoggingDecorator {
    inner: Arc<dyn Notifier>,
}

impl Notifier for LoggingDecorator {
    fn send(&self) -> String {
        let msg = self.inner.send();
        format!("log:{}", msg)
    }
}

#[test]
fn decorator_pattern() {
    let p = ServiceCollection::new()
        .singleton::<dyn Notifier>(|_| Arc::new(BaseNotifier))
        .build()
        .unwrap();
    let inner: Arc<dyn Notifier> = p.get();
    let decorator = LoggingDecorator { inner };
    assert_eq!(decorator.send(), "log:base");
}

// ── Chain of Responsibility ──

trait Handler: Send + Sync {
    fn handle(&self, req: &str) -> Option<String>;
}

struct AuthHandler;
impl Handler for AuthHandler {
    fn handle(&self, req: &str) -> Option<String> {
        if req.contains("auth") {
            Some("authenticated".to_string())
        } else {
            None
        }
    }
}

struct LogHandler;
impl Handler for LogHandler {
    fn handle(&self, req: &str) -> Option<String> {
        Some(format!("logged:{}", req))
    }
}

#[test]
fn chain_of_responsibility() {
    let p = ServiceCollection::new()
        .keyed::<dyn Handler>("auth", |_| Arc::new(AuthHandler))
        .keyed::<dyn Handler>("log", |_| Arc::new(LogHandler))
        .build()
        .unwrap();
    let auth: Arc<dyn Handler> = p.get_keyed("auth");
    let log: Arc<dyn Handler> = p.get_keyed("log");
    assert_eq!(auth.handle("auth_ok"), Some("authenticated".into()));
    assert!(auth.handle("other").is_none());
    assert_eq!(log.handle("test"), Some("logged:test".into()));
}

// ── Observer Pattern ──

trait Observer: Send + Sync {
    fn notify(&self, msg: &str) -> String;
}

struct EmailObserver {
    addr: &'static str,
}
impl Observer for EmailObserver {
    fn notify(&self, msg: &str) -> String {
        format!("email({}):{}", self.addr, msg)
    }
}

struct SmsObserver {
    phone: &'static str,
}
impl Observer for SmsObserver {
    fn notify(&self, msg: &str) -> String {
        format!("sms({}):{}", self.phone, msg)
    }
}

#[test]
fn observer_pattern() {
    let p = ServiceCollection::new()
        .keyed::<dyn Observer>("email", |_| Arc::new(EmailObserver { addr: "a@b" }))
        .keyed::<dyn Observer>("sms", |_| Arc::new(SmsObserver { phone: "555" }))
        .build()
        .unwrap();
    let email: Arc<dyn Observer> = p.get_keyed("email");
    let sms: Arc<dyn Observer> = p.get_keyed("sms");
    assert_eq!(email.notify("hi"), "email(a@b):hi");
    assert_eq!(sms.notify("alert"), "sms(555):alert");
}

// ── Composite Pattern ──

trait Component: Send + Sync {
    fn render(&self) -> Vec<&'static str>;
}

struct LeafNode {
    name: &'static str,
}
impl Component for LeafNode {
    fn render(&self) -> Vec<&'static str> {
        vec![self.name]
    }
}

struct GroupNode {
    children: Vec<Arc<dyn Component>>,
}
impl Component for GroupNode {
    fn render(&self) -> Vec<&'static str> {
        self.children.iter().flat_map(|c| c.render()).collect()
    }
}

#[test]
fn composite_pattern() {
    let p = ServiceCollection::new()
        .keyed::<dyn Component>("leaf_a", |_| Arc::new(LeafNode { name: "A" }))
        .keyed::<dyn Component>("leaf_b", |_| Arc::new(LeafNode { name: "B" }))
        .build()
        .unwrap();
    let a: Arc<dyn Component> = p.get_keyed("leaf_a");
    let b: Arc<dyn Component> = p.get_keyed("leaf_b");
    assert_eq!(a.render(), vec!["A"]);
    assert_eq!(b.render(), vec!["B"]);
    let group = GroupNode {
        children: vec![a, b],
    };
    assert_eq!(group.render(), vec!["A", "B"]);
}

// ── Proxy Pattern ──

trait Image: Send + Sync {
    fn display(&self) -> &'static str;
}

struct RealImage {
    #[allow(dead_code)]
    name: &'static str,
}
impl Image for RealImage {
    fn display(&self) -> &'static str {
        "real_image"
    }
}

#[test]
fn proxy_pattern() {
    let p = ServiceCollection::new()
        .singleton::<dyn Image>(|_| Arc::new(RealImage { name: "photo" }))
        .build()
        .unwrap();
    let img: Arc<dyn Image> = p.get();
    assert_eq!(img.display(), "real_image");
}

// ── Plugin Architecture ──

trait Plugin: Send + Sync {
    fn plugin_name(&self) -> &'static str;
    fn execute(&self) -> i32;
}

struct MathPlugin;
impl Plugin for MathPlugin {
    fn plugin_name(&self) -> &'static str {
        "math"
    }
    fn execute(&self) -> i32 {
        42
    }
}

struct IoPlugin;
impl Plugin for IoPlugin {
    fn plugin_name(&self) -> &'static str {
        "io"
    }
    fn execute(&self) -> i32 {
        7
    }
}

#[test]
fn plugin_architecture() {
    let p = ServiceCollection::new()
        .keyed::<dyn Plugin>("math", |_| Arc::new(MathPlugin))
        .keyed::<dyn Plugin>("io", |_| Arc::new(IoPlugin))
        .build()
        .unwrap();
    let math: Arc<dyn Plugin> = p.get_keyed("math");
    let io: Arc<dyn Plugin> = p.get_keyed("io");
    assert_eq!(math.plugin_name(), "math");
    assert_eq!(math.execute(), 42);
    assert_eq!(io.plugin_name(), "io");
    assert_eq!(io.execute(), 7);
}

// ── Feature Toggle ──

trait Feature: Send + Sync {
    fn is_enabled(&self) -> bool;
}

struct NewCheckout;
impl Feature for NewCheckout {
    fn is_enabled(&self) -> bool {
        true
    }
}

struct OldCheckout;
impl Feature for OldCheckout {
    fn is_enabled(&self) -> bool {
        false
    }
}

#[test]
fn feature_toggle() {
    let p = ServiceCollection::new()
        .keyed::<dyn Feature>("new_checkout", |_| Arc::new(NewCheckout))
        .keyed::<dyn Feature>("old_checkout", |_| Arc::new(OldCheckout))
        .build()
        .unwrap();
    let new: Arc<dyn Feature> = p.get_keyed("new_checkout");
    let old: Arc<dyn Feature> = p.get_keyed("old_checkout");
    assert!(new.is_enabled());
    assert!(!old.is_enabled());
}

// ── Pipeline Pattern ──

trait Stage: Send + Sync {
    fn process(&self, input: &str) -> String;
}

struct UpperStage;
impl Stage for UpperStage {
    fn process(&self, input: &str) -> String {
        input.to_uppercase()
    }
}

struct TrimStage;
impl Stage for TrimStage {
    fn process(&self, input: &str) -> String {
        input.trim().to_string()
    }
}

#[test]
fn pipeline_pattern() {
    let p = ServiceCollection::new()
        .keyed::<dyn Stage>("upper", |_| Arc::new(UpperStage))
        .keyed::<dyn Stage>("trim", |_| Arc::new(TrimStage))
        .build()
        .unwrap();
    let upper: Arc<dyn Stage> = p.get_keyed("upper");
    let trim: Arc<dyn Stage> = p.get_keyed("trim");
    assert_eq!(upper.process("hello"), "HELLO");
    assert_eq!(trim.process("  spaced  "), "spaced");
}

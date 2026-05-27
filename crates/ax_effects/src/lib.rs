use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub struct EffectSet {
    inner: BTreeSet<String>,
}

impl EffectSet {
    pub fn insert(&mut self, effect: impl Into<String>) {
        self.inner.insert(effect.into());
    }

    pub fn contains(&self, effect: &str) -> bool {
        self.inner.contains(effect)
    }

    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.inner.iter()
    }
}

pub const CORE_EFFECTS: &[&str] = &[
    "io.stdout",
    "io.stderr",
    "io.stdin",
    "fs.read",
    "fs.write",
    "crypto.hash",
    "crypto.random",
    "env.read",
    "env.write",
    "process.exec",
    "net.listen",
    "net.read",
    "net.write",
    "heap.alloc",
    "heap.free",
    "async.cancel",
    "async.detach",
    "panic",
];

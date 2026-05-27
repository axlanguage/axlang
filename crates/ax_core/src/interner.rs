use std::collections::HashMap;

#[derive(Default)]
pub struct Interner {
    ids: HashMap<String, usize>,
    values: Vec<String>,
}

impl Interner {
    pub fn intern(&mut self, value: &str) -> usize {
        if let Some(id) = self.ids.get(value) {
            return *id;
        }
        let id = self.values.len();
        self.values.push(value.to_string());
        self.ids.insert(value.to_string(), id);
        id
    }

    pub fn get(&self, id: usize) -> Option<&str> {
        self.values.get(id).map(String::as_str)
    }
}

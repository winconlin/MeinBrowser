use super::*;
use std::collections::BTreeSet;

pub(super) struct Metadata<'a> {
    required: BTreeMap<DefinitionId, bool>,
    enums: BTreeMap<DefinitionId, BTreeSet<&'a str>>,
}
impl<'a> Metadata<'a> {
    pub fn build(index: &Index<'_>, members: &Members<'a>, work: &Work<'_>) -> Result<Self, Error> {
        let mut dictionaries = BTreeMap::new();
        let mut enums = BTreeMap::new();
        for (id, definition) in index.definitions() {
            work.spend(1, Some(&definition.location))?;
            if definition.kind == DefinitionKind::Dictionary {
                let mut required = false;
                for member in members.members(id).expect("Dictionaryfelder") {
                    work.spend(1, Some(&member.location))?;
                    required |= member.required;
                }
                dictionaries.insert(id, (definition.parent, required));
            } else if definition.kind == DefinitionKind::Enum {
                let mut values = BTreeSet::new();
                for value in members.values(id).expect("Enumwerte") {
                    work.spend(1 + value.value.len() as u64, Some(&value.location))?;
                    values.insert(value.value);
                }
                enums.insert(id, values);
            }
        }
        let mut required = BTreeMap::new();
        for &root in dictionaries.keys() {
            let mut cursor = Some(root);
            let mut pending = Vec::new();
            let mut inherited = false;
            while let Some(id) = cursor {
                work.spend(1, None)?;
                if let Some(&known) = required.get(&id) {
                    inherited = known;
                    break;
                }
                let &(parent, own) = dictionaries.get(&id).expect("Dictionary");
                pending.push((id, own));
                cursor = parent;
            }
            while let Some((id, own)) = pending.pop() {
                work.spend(1, None)?;
                inherited |= own;
                required.insert(id, inherited);
            }
        }
        Ok(Self { required, enums })
    }
    pub fn required(&self, id: DefinitionId) -> bool {
        self.required[&id]
    }
    pub fn enum_value(&self, id: DefinitionId, raw: &str) -> bool {
        self.enums.get(&id).expect("Enum").contains(raw)
    }
}

//! Lecture en flux de `nodes.json`.
//!
//! Le fichier n'est jamais chargé en entier : les étapes et les actions sont
//! lues une par une, converties en structures compactes, puis la valeur JSON
//! temporaire est libérée. Les clés d'étape et d'action ne servent que pendant
//! la lecture, pour résoudre les références croisées.
//!
//! La lecture est tolérante : un champ absent ou d'un type inattendu est ignoré
//! au lieu de faire échouer toute l'histoire.

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;

use crate::model::{
    Action, ActionOption, Condition, InventoryItem, ItemUpdate, Operand, OptionIndex,
    Span, Stage, StageId, Story, Transition,
};

#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// Fichier valide mais sans aucune étape.
    Empty,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "lecture impossible : {e}"),
            ParseError::Json(e) => write!(f, "JSON invalide : {e}"),
            ParseError::Empty => write!(f, "aucune étape"),
        }
    }
}

impl std::error::Error for ParseError {}

pub fn load_story(path: &Path) -> Result<Story, ParseError> {
    let file = File::open(path).map_err(ParseError::Io)?;
    parse_story(BufReader::with_capacity(64 * 1024, file))
}

pub fn parse_story<R: Read>(reader: R) -> Result<Story, ParseError> {
    let mut builder = Builder::default();
    let mut de = serde_json::Deserializer::from_reader(reader);
    de.deserialize_map(RootVisitor { builder: &mut builder })
        .map_err(ParseError::Json)?;
    de.end().map_err(ParseError::Json)?;
    builder.finish()
}

/// Attribue un identifiant dense à chaque clé, dans l'ordre de première apparition.
#[derive(Default)]
struct Interner {
    ids: HashMap<Box<str>, u32>,
}

impl Interner {
    fn id(&mut self, key: &str) -> u32 {
        if let Some(&id) = self.ids.get(key) {
            return id;
        }
        let id = self.ids.len() as u32;
        self.ids.insert(key.into(), id);
        id
    }

    fn len(&self) -> usize {
        self.ids.len()
    }
}

/// Option dont l'étape n'est connue que par son identifiant interné.
struct PendingOption {
    stage: u32,
    conditions: Span,
}

#[derive(Default)]
struct Builder {
    story: Story,
    stage_keys: Interner,
    action_keys: Interner,
    /// Étapes indexées par identifiant interné ; `None` tant qu'elles ne sont pas définies.
    stages: Vec<Option<Stage>>,
    pending_options: Vec<PendingOption>,
}

impl Builder {
    fn transition(&mut self, value: &Value) -> Option<Transition> {
        let action = value.get("action")?.as_str()?;
        let index = if let Some(n) = int(value.get("index")) {
            OptionIndex::Fixed(n)
        } else if let Some(item) = item(value.get("indexItem")) {
            OptionIndex::Item(item)
        } else {
            OptionIndex::Fixed(0)
        };
        Some(Transition { action: self.action_keys.id(action), index })
    }

    fn add_stage(&mut self, key: &str, value: &Value) {
        let control = value.get("control");
        let updates_start = self.story.updates.len() as u32;
        if let Some(items) = value.get("items").and_then(Value::as_array) {
            for update in items {
                let (Some(item), Some(kind)) = (item(update.get("item")), int(update.get("type")))
                else {
                    continue;
                };
                let operand = if let Some(n) = int(update.get("number")) {
                    Operand::Number(n)
                } else if let Some(other) = self::item(update.get("assignItem")) {
                    Operand::Item(other)
                } else if is_true(update.get("playingTime")) {
                    Operand::PlayingTime
                } else {
                    Operand::Number(0)
                };
                self.story.updates.push(ItemUpdate { item, kind: kind as u8, operand });
            }
        }
        let stage = Stage {
            audio: self.non_empty_str(value.get("audio")),
            image: self.non_empty_str(value.get("image")),
            ok: value.get("ok").and_then(|t| self.transition(t)),
            home: value.get("home").and_then(|t| self.transition(t)),
            autoplay: is_true(control.and_then(|c| c.get("autoplay"))),
            ok_enabled: is_true(control.and_then(|c| c.get("ok"))),
            inventory_reset: is_true(value.get("inventoryReset")),
            updates: Span {
                start: updates_start,
                len: self.story.updates.len() as u32 - updates_start,
            },
        };
        let id = self.stage_keys.id(key) as usize;
        if self.stages.len() <= id {
            self.stages.resize(id + 1, None);
        }
        self.stages[id] = Some(stage);
    }

    fn add_action(&mut self, key: &str, value: &Value) {
        let options_start = self.pending_options.len() as u32;
        for option in value.as_array().map(Vec::as_slice).unwrap_or_default() {
            let Some(stage_key) = option.get("stage").and_then(Value::as_str) else {
                // Option inutilisable, gardée pour conserver les index de Telmi Sync.
                self.pending_options.push(PendingOption { stage: u32::MAX, conditions: Span::default() });
                continue;
            };
            let conditions_start = self.story.conditions.len() as u32;
            for condition in option.get("conditions").and_then(Value::as_array).into_iter().flatten() {
                let (Some(item), Some(comparator)) =
                    (item(condition.get("item")), int(condition.get("comparator")))
                else {
                    continue;
                };
                let operand = if let Some(n) = int(condition.get("number")) {
                    Operand::Number(n)
                } else if let Some(other) = self::item(condition.get("compareItem")) {
                    Operand::Item(other)
                } else {
                    Operand::Number(0)
                };
                self.story.conditions.push(Condition { item, comparator: comparator as u8, operand });
            }
            let stage = self.stage_keys.id(stage_key);
            self.pending_options.push(PendingOption {
                stage,
                conditions: Span {
                    start: conditions_start,
                    len: self.story.conditions.len() as u32 - conditions_start,
                },
            });
        }
        let action = Action {
            key: self.story.push_str(key),
            options: Span {
                start: options_start,
                len: self.pending_options.len() as u32 - options_start,
            },
        };
        let id = self.action_keys.id(key) as usize;
        if self.story.actions.len() <= id {
            self.story.actions.resize(id + 1, None);
        }
        self.story.actions[id] = Some(action);
    }

    fn set_inventory(&mut self, value: &Value) {
        for entry in value.as_array().map(Vec::as_slice).unwrap_or_default() {
            let item = InventoryItem {
                initial: int(entry.get("initialNumber")).unwrap_or(0),
                max: int(entry.get("maxNumber")).unwrap_or(0),
                display: int(entry.get("display")).unwrap_or(0).clamp(0, 2) as u8,
                image: self.non_empty_str(entry.get("image")),
            };
            self.story.inventory.push(item);
        }
    }

    fn non_empty_str(&mut self, value: Option<&Value>) -> Option<crate::model::StrRef> {
        match value.and_then(Value::as_str) {
            Some(s) if !s.is_empty() => Some(self.story.push_str(s)),
            _ => None,
        }
    }

    fn finish(mut self) -> Result<Story, ParseError> {
        // Identifiants internés → index denses des seules étapes définies.
        let mut dense: Vec<Option<StageId>> = vec![None; self.stage_keys.len()];
        let mut stages = Vec::with_capacity(self.stages.iter().flatten().count());
        for (interned, stage) in self.stages.into_iter().enumerate() {
            if let Some(stage) = stage {
                dense[interned] = Some(stages.len() as StageId);
                stages.push(stage);
            }
        }
        if stages.is_empty() {
            return Err(ParseError::Empty);
        }
        self.story.stages = stages;
        self.story.options = self
            .pending_options
            .into_iter()
            .map(|o| ActionOption {
                stage: dense.get(o.stage as usize).copied().flatten(),
                conditions: o.conditions,
            })
            .collect();
        // Actions référencées par une transition mais jamais définies.
        self.story.actions.resize(self.action_keys.len(), None);

        let story = &mut self.story;
        story.arena.shrink_to_fit();
        story.actions.shrink_to_fit();
        story.conditions.shrink_to_fit();
        story.updates.shrink_to_fit();
        story.inventory.shrink_to_fit();
        Ok(self.story)
    }
}

fn int(value: Option<&Value>) -> Option<i32> {
    // Même conversion que le `(int)` de Telmi : troncature, saturée.
    value.and_then(Value::as_f64).map(|n| n as i32)
}

fn item(value: Option<&Value>) -> Option<u16> {
    int(value).and_then(|n| u16::try_from(n).ok())
}

fn is_true(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

struct RootVisitor<'b> {
    builder: &'b mut Builder,
}

impl<'de> Visitor<'de> for RootVisitor<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("un objet nodes.json")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "stages" => map.next_value_seed(Entries { builder: self.builder, kind: Kind::Stage })?,
                "actions" => map.next_value_seed(Entries { builder: self.builder, kind: Kind::Action })?,
                "startAction" => {
                    let value: Value = map.next_value()?;
                    self.builder.story.start = self.builder.transition(&value);
                }
                "inventory" => {
                    let value: Value = map.next_value()?;
                    self.builder.set_inventory(&value);
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Stage,
    Action,
}

/// Parcourt un objet `{clé: valeur}` en ne gardant qu'une entrée à la fois en mémoire.
struct Entries<'b> {
    builder: &'b mut Builder,
    kind: Kind,
}

impl<'de> DeserializeSeed<'de> for Entries<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Entries<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("un objet d'étapes ou d'actions")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some((key, value)) = map.next_entry::<String, Value>()? {
            match self.kind {
                Kind::Stage => self.builder.add_stage(&key, &value),
                Kind::Action => self.builder.add_action(&key, &value),
            }
        }
        Ok(())
    }

    // Toute autre forme (null, tableau…) est ignorée.
    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Story {
        parse_story(json.as_bytes()).expect("histoire valide")
    }

    #[test]
    fn resolves_forward_references() {
        let story = parse(
            r#"{
                "startAction": {"action": "a", "index": 0},
                "stages": {
                    "s1": {"audio": "1.mp3", "image": null, "ok": {"action": "b"}, "home": null,
                           "control": {"autoplay": true, "ok": true}},
                    "s2": {"audio": "", "image": "2.png", "ok": null, "home": {"action": "a"}}
                },
                "actions": {"a": [{"stage": "s1"}], "b": [{"stage": "s2"}, {"stage": "missing"}]}
            }"#,
        );
        assert_eq!(story.stage_count(), 2);
        let start = story.start().unwrap();
        let a = story.action(start.action).unwrap();
        let s1 = story.stage(story.options(a)[0].stage.unwrap()).unwrap();
        assert_eq!(story.str(s1.audio.unwrap()), "1.mp3");
        assert!(s1.autoplay && s1.ok_enabled);
        let b = story.action(s1.ok.unwrap().action).unwrap();
        assert_eq!(story.str(b.key), "b");
        let options = story.options(b);
        let s2 = story.stage(options[0].stage.unwrap()).unwrap();
        assert!(s2.audio.is_none(), "chaîne vide = pas d'audio");
        assert_eq!(story.str(s2.image.unwrap()), "2.png");
        assert!(options[1].stage.is_none(), "étape absente");
    }

    #[test]
    fn tolerates_unexpected_types() {
        let story = parse(
            r#"{"startAction": 3, "stages": {"s": {"audio": 12, "ok": false, "control": null,
                "items": [{"item": -1, "type": 0}, {"item": 0, "type": 2, "number": "x"}]}},
                "actions": null, "notes": {"x": [1, 2]}}"#,
        );
        assert!(story.start().is_none());
        let stage = story.stage(0).unwrap();
        assert!(stage.audio.is_none() && stage.ok.is_none() && !stage.autoplay);
        assert_eq!(story.updates(stage).len(), 1);
    }

    #[test]
    fn rejects_story_without_stages() {
        assert!(matches!(parse_story(&b"{}"[..]), Err(ParseError::Empty)));
        assert!(matches!(parse_story(&b"{\"stages\": "[..]), Err(ParseError::Json(_))));
    }

    #[test]
    fn keeps_undefined_actions_as_none() {
        let story = parse(
            r#"{"stages": {"s": {"ok": {"action": "ghost", "indexItem": 1}}}, "actions": {}}"#,
        );
        let ok = story.stage(0).unwrap().ok.unwrap();
        assert_eq!(ok.index, OptionIndex::Item(1));
        assert!(story.action(ok.action).is_none());
    }

    #[test]
    fn reads_inventory_and_conditions() {
        let story = parse(
            r#"{"inventory": [{"initialNumber": 1, "maxNumber": 5, "display": 1, "image": "i.png"}],
                "stages": {"s": {}},
                "actions": {"a": [{"stage": "s", "conditions": [
                    {"item": 0, "comparator": 3, "number": 2},
                    {"item": 0, "comparator": 2, "compareItem": 0}]}]}}"#,
        );
        assert_eq!(story.inventory().len(), 1);
        assert_eq!(story.inventory()[0].max, 5);
        let a = story.action(story.action_by_key("a").unwrap()).unwrap();
        let conditions = story.conditions(&story.options(a)[0]);
        assert_eq!(conditions[0].operand, Operand::Number(2));
        assert_eq!(conditions[1].operand, Operand::Item(0));
    }
}

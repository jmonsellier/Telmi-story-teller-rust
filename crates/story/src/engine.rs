//! Navigation dans une histoire : actions, options, étapes et inventaire.
//!
//! Reprend la logique de `stories_reader.h` de Telmi OS, avec trois garanties
//! en plus :
//! - aucune donnée du fichier ne peut provoquer un accès hors limites ou une
//!   division par zéro ;
//! - une boucle d'étapes automatiques sans audio est coupée au lieu de faire
//!   déborder la pile ;
//! - toute référence cassée ramène proprement à l'écran titre.

use crate::model::{
    ActionId, Condition, ItemUpdate, Operand, OptionIndex, Stage, StageId, Story, Transition,
};

/// Nombre maximal d'étapes automatiques enchaînées sans intervention.
const MAX_AUTO_HOPS: usize = 1024;

/// Résultat d'une commande de navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Une étape est à présenter (audio, image, ou les deux).
    Stage(StageId),
    /// Fin de l'histoire ou donnée cassée : retour à l'écran titre.
    Reset,
}

/// Position dans une histoire, au format des sauvegardes Telmi.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resume {
    pub action_key: String,
    pub option_index: i32,
    pub playing_time: i64,
    pub inventory: Vec<i32>,
}

pub struct Player {
    story: Story,
    action: Option<ActionId>,
    option_index: i32,
    option_count: i32,
    stage: Option<StageId>,
    inventory: Vec<i32>,
    playing_time: i64,
    last_tick: i64,
    rng: u64,
}

/// Issue interne d'une étape : terminée, ou suite automatique à suivre.
enum Flow {
    Done(Step),
    Follow(Option<Transition>),
}

impl Player {
    pub fn new(story: Story, seed: u64) -> Self {
        Player {
            story,
            action: None,
            option_index: 0,
            option_count: 0,
            stage: None,
            inventory: Vec::new(),
            playing_time: 0,
            last_tick: 0,
            rng: seed | 1,
        }
    }

    pub fn story(&self) -> &Story {
        &self.story
    }

    pub fn current_stage(&self) -> Option<&Stage> {
        self.stage.and_then(|id| self.story.stage(id))
    }

    pub fn inventory(&self) -> &[i32] {
        &self.inventory
    }

    /// Démarre l'histoire, depuis une sauvegarde si elle est encore valide.
    pub fn start(&mut self, resume: Option<&Resume>, now: i64) -> Step {
        self.playing_time = 0;
        self.last_tick = now;
        let size = self.story.inventory().len();
        self.inventory = match resume {
            Some(r) if r.inventory.len() == size => r.inventory.clone(),
            _ => self.story.inventory().iter().map(|i| i.initial).collect(),
        };

        if let Some(r) = resume {
            if let Some(action) = self.story.action_by_key(&r.action_key) {
                self.playing_time = r.playing_time;
                self.action = Some(action);
                self.option_index = r.option_index;
                let flow = self.load_action(now);
                return self.drive(flow, now);
            }
        }
        // Sauvegarde absente ou périmée : on repart du début.
        self.inventory = self.story.inventory().iter().map(|i| i.initial).collect();
        let start = self.story.start();
        self.follow(start, now)
    }

    /// Bouton OK pendant une étape (transition `ok`).
    pub fn ok(&mut self, now: i64) -> Step {
        let next = self.current_stage().and_then(|s| s.ok);
        self.follow(next, now)
    }

    /// Bouton Home pendant une étape (transition `home`).
    pub fn home(&mut self, now: i64) -> Step {
        let next = self.current_stage().and_then(|s| s.home);
        self.follow(next, now)
    }

    /// Option suivante (+1) ou précédente (-1) de l'action en cours.
    pub fn select(&mut self, direction: i32, now: i64) -> Step {
        if self.action.is_none() {
            return Step::Reset;
        }
        let flow = self.read_action(direction, now);
        self.drive(flow, now)
    }

    /// État à sauvegarder pour reprendre plus tard.
    pub fn snapshot(&mut self, now: i64) -> Resume {
        Resume {
            action_key: self
                .action
                .and_then(|a| self.story.action(a))
                .map(|a| self.story.str(a.key).to_owned())
                .unwrap_or_default(),
            option_index: self.option_index,
            playing_time: self.playing_time(now),
            inventory: self.inventory.clone(),
        }
    }

    fn playing_time(&mut self, now: i64) -> i64 {
        self.playing_time += now - self.last_tick;
        self.last_tick = now;
        self.playing_time
    }

    fn follow(&mut self, transition: Option<Transition>, now: i64) -> Step {
        self.drive(Flow::Follow(transition), now)
    }

    fn drive(&mut self, mut flow: Flow, now: i64) -> Step {
        for _ in 0..MAX_AUTO_HOPS {
            match flow {
                Flow::Done(step) => return step,
                Flow::Follow(None) => return Step::Reset,
                Flow::Follow(Some(t)) => {
                    self.action = Some(t.action);
                    self.option_index = match t.index {
                        OptionIndex::Fixed(i) => i,
                        OptionIndex::Item(item) => self.item(item),
                    };
                    flow = self.load_action(now);
                }
            }
        }
        Step::Reset
    }

    fn load_action(&mut self, now: i64) -> Flow {
        let Some(action) = self.action.and_then(|a| self.story.action(a)).copied() else {
            return Flow::Done(Step::Reset);
        };
        let options = self.story.options(&action);
        if options.is_empty() {
            return Flow::Done(Step::Reset);
        }
        self.option_count = options.len() as i32;

        if self.option_index < 0 {
            let candidates: Vec<i32> = options
                .iter()
                .enumerate()
                .filter(|(_, o)| self.story.inventory().is_empty() || self.conditions_hold(self.story.conditions(o)))
                .map(|(i, _)| i as i32)
                .collect();
            if candidates.is_empty() {
                return Flow::Done(Step::Reset);
            }
            self.option_index = candidates[self.random(candidates.len())];
        }
        self.option_index = self.option_index.min(self.option_count - 1);
        self.read_action(0, now)
    }

    fn read_action(&mut self, direction: i32, now: i64) -> Flow {
        let Some(action) = self.action.and_then(|a| self.story.action(a)).copied() else {
            return Flow::Done(Step::Reset);
        };
        let count = self.story.options(&action).len() as i32;
        if count == 0 {
            return Flow::Done(Step::Reset);
        }
        self.option_count = count;
        let step = if direction == 0 { 1 } else { direction.signum() };
        self.option_index += direction;
        let mut first = None;
        loop {
            self.option_index = self.option_index.rem_euclid(count);
            if first == Some(self.option_index) {
                return Flow::Done(Step::Reset);
            }
            first.get_or_insert(self.option_index);

            let option = self.story.options(&action)[self.option_index as usize];
            let usable = option.stage.is_some()
                && (self.story.inventory().is_empty()
                    || self.conditions_hold(self.story.conditions(&option)));
            match option.stage {
                Some(stage) if usable => return self.enter(stage, now),
                _ => self.option_index += step,
            }
        }
    }

    fn enter(&mut self, id: StageId, now: i64) -> Flow {
        let Some(stage) = self.story.stage(id).copied() else {
            return Flow::Done(Step::Reset);
        };
        self.stage = Some(id);
        if !self.story.inventory().is_empty() {
            if stage.inventory_reset {
                self.inventory = self.story.inventory().iter().map(|i| i.initial).collect();
            }
            let updates: Vec<ItemUpdate> = self.story.updates(&stage).to_vec();
            for update in updates {
                self.apply(update, now);
            }
        }
        if stage.audio.is_none() && stage.autoplay {
            // Étape de passage : Telmi enchaîne aussitôt sur `ok`.
            return Flow::Follow(stage.ok);
        }
        Flow::Done(Step::Stage(id))
    }

    fn item(&self, item: u16) -> i32 {
        self.inventory.get(item as usize).copied().unwrap_or(0)
    }

    fn operand(&mut self, operand: Operand, now: i64) -> i32 {
        match operand {
            Operand::Number(n) => n,
            Operand::Item(item) => self.item(item),
            Operand::PlayingTime => self.playing_time(now).clamp(0, i32::MAX as i64) as i32,
        }
    }

    fn apply(&mut self, update: ItemUpdate, now: i64) {
        let Some(max) = self.story.inventory().get(update.item as usize).map(|i| i.max) else {
            return;
        };
        let number = self.operand(update.operand, now);
        let current = self.item(update.item);
        let value = match update.kind {
            0 => current.saturating_add(number),
            1 => current.saturating_sub(number),
            2 => number,
            3 => current.saturating_mul(number),
            4 => current.checked_div(number).unwrap_or(current),
            5 => current.checked_rem(number).unwrap_or(current),
            _ => current,
        };
        // Même ordre que Telmi : plafond d'abord, plancher ensuite.
        self.inventory[update.item as usize] = if value > max { max } else { value.max(0) };
    }

    fn conditions_hold(&self, conditions: &[Condition]) -> bool {
        conditions.iter().all(|c| {
            let value = self.item(c.item);
            let target = match c.operand {
                Operand::Number(n) => n,
                Operand::Item(item) => self.item(item),
                Operand::PlayingTime => 0,
            };
            match c.comparator {
                0 => value < target,
                1 => value <= target,
                2 => value == target,
                3 => value > target,
                4 => value >= target,
                5 => value != target,
                _ => false,
            }
        })
    }

    /// Tirage dans `0..n` (xorshift64, suffisant pour choisir une option).
    fn random(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_story;

    fn player(json: &str) -> Player {
        Player::new(parse_story(json.as_bytes()).unwrap(), 42)
    }

    fn audio(p: &Player) -> &str {
        let stage = p.current_stage().unwrap();
        p.story().str(stage.audio.unwrap())
    }

    const MENU: &str = r#"{
        "startAction": {"action": "intro", "index": 0},
        "stages": {
            "intro": {"audio": "intro.mp3", "ok": {"action": "menu"}, "control": {"autoplay": true, "ok": true}},
            "a": {"audio": "a.mp3", "image": "a.png", "ok": {"action": "story_a"}, "home": {"action": "intro"}},
            "b": {"audio": "b.mp3", "image": "b.png", "ok": {"action": "story_b"}, "home": {"action": "intro"}},
            "pass": {"audio": null, "ok": {"action": "menu", "index": 1}, "control": {"autoplay": true}},
            "loop": {"ok": {"action": "loop"}, "control": {"autoplay": true}}
        },
        "actions": {
            "intro": [{"stage": "intro"}],
            "menu": [{"stage": "a"}, {"stage": "b"}],
            "story_a": [{"stage": "pass"}],
            "story_b": [{"stage": "loop"}]
        }
    }"#;

    #[test]
    fn navigates_menu_and_wraps_options() {
        let mut p = player(MENU);
        assert!(matches!(p.start(None, 0), Step::Stage(_)));
        assert_eq!(audio(&p), "intro.mp3");
        p.ok(0);
        assert_eq!(audio(&p), "a.mp3");
        p.select(1, 0);
        assert_eq!(audio(&p), "b.mp3");
        p.select(1, 0);
        assert_eq!(audio(&p), "a.mp3", "retour au début de la liste");
        p.select(-1, 0);
        assert_eq!(audio(&p), "b.mp3");
    }

    #[test]
    fn follows_silent_autoplay_stages() {
        let mut p = player(MENU);
        p.start(None, 0);
        p.ok(0);
        p.ok(0); // a → story_a → pass (sans audio) → menu[1]
        assert_eq!(audio(&p), "b.mp3");
    }

    #[test]
    fn breaks_infinite_silent_loops() {
        let mut p = player(MENU);
        p.start(None, 0);
        p.ok(0);
        p.select(1, 0);
        assert_eq!(p.ok(0), Step::Reset);
    }

    #[test]
    fn home_and_missing_transitions() {
        let mut p = player(MENU);
        p.start(None, 0);
        assert_eq!(p.home(0), Step::Reset, "intro n'a pas de transition home");
        p.start(None, 0);
        p.ok(0);
        p.home(0);
        assert_eq!(audio(&p), "intro.mp3");
    }

    #[test]
    fn resumes_from_snapshot_or_restarts() {
        let mut p = player(MENU);
        p.start(None, 0);
        p.ok(0);
        p.select(1, 0);
        let saved = p.snapshot(30);
        assert_eq!(saved.action_key, "menu");
        assert_eq!(saved.option_index, 1);
        assert_eq!(saved.playing_time, 30);

        let mut q = player(MENU);
        q.start(Some(&saved), 100);
        assert_eq!(audio(&q), "b.mp3");

        let stale = Resume { action_key: "disparue".into(), ..saved };
        q.start(Some(&stale), 100);
        assert_eq!(audio(&q), "intro.mp3");
    }

    const INVENTORY: &str = r#"{
        "inventory": [{"initialNumber": 0, "maxNumber": 3}, {"initialNumber": 2, "maxNumber": 9}],
        "startAction": {"action": "go"},
        "stages": {
            "gain": {"audio": "gain.mp3", "ok": {"action": "check", "index": -1},
                     "items": [{"item": 0, "type": 0, "number": 5}, {"item": 1, "type": 4, "number": 0},
                               {"item": 7, "type": 0, "number": 1}]},
            "rich": {"audio": "rich.mp3", "ok": {"action": "go"}, "inventoryReset": true},
            "poor": {"audio": "poor.mp3"}
        },
        "actions": {
            "go": [{"stage": "gain"}],
            "check": [{"stage": "poor", "conditions": [{"item": 0, "comparator": 0, "number": 3}]},
                      {"stage": "rich", "conditions": [{"item": 0, "comparator": 2, "compareItem": 0},
                                                       {"item": 0, "comparator": 4, "number": 3}]}]
        }
    }"#;

    #[test]
    fn applies_inventory_safely() {
        let mut p = player(INVENTORY);
        p.start(None, 0);
        assert_eq!(audio(&p), "gain.mp3");
        assert_eq!(p.inventory(), &[3, 2], "plafonné à 3, division par zéro ignorée");
        p.ok(0); // tirage parmi les options valides : seule « rich » passe
        assert_eq!(audio(&p), "rich.mp3");
        assert_eq!(p.inventory(), &[0, 2], "inventoryReset");
    }

    #[test]
    fn indexes_options_from_inventory() {
        let mut p = player(
            r#"{"inventory": [{"initialNumber": 1, "maxNumber": 5}],
                "startAction": {"action": "m", "indexItem": 0},
                "stages": {"x": {"audio": "x.mp3"}, "y": {"audio": "y.mp3"}},
                "actions": {"m": [{"stage": "x"}, {"stage": "y"}]}}"#,
        );
        p.start(None, 0);
        assert_eq!(audio(&p), "y.mp3");
    }

    #[test]
    fn resets_when_no_option_is_usable() {
        let mut p = player(
            r#"{"inventory": [{"initialNumber": 0, "maxNumber": 5}],
                "startAction": {"action": "m"},
                "stages": {"x": {"audio": "x.mp3"}},
                "actions": {"m": [{"stage": "x", "conditions": [{"item": 0, "comparator": 3, "number": 1}]},
                                  {"stage": "nowhere"}]}}"#,
        );
        assert_eq!(p.start(None, 0), Step::Reset);
    }
}

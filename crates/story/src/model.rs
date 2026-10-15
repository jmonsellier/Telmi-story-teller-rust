//! Représentation compacte d'une histoire Telmi.
//!
//! Telmi OS garde tout `nodes.json` sous forme d'arbre cJSON : environ 70 octets
//! par valeur JSON, soit 46 Mo pour un fichier de 10 Mo. Ici, chaque étape tient
//! dans une structure fixe et toutes les chaînes vivent dans une seule arène,
//! ce qui divise l'empreinte par dix environ.

use std::ops::Range;

/// Index d'une étape dans [`Story::stages`].
pub type StageId = u32;
/// Index d'une action dans [`Story::actions`].
pub type ActionId = u32;

/// Tranche contiguë d'un des tableaux de [`Story`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub len: u32,
}

impl Span {
    pub fn range(self) -> Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }
}

/// Chaîne stockée dans l'arène de l'histoire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrRef {
    start: u32,
    len: u32,
}

/// Choix de l'option dans l'action cible d'une transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionIndex {
    /// Index fixe. Une valeur négative demande un tirage au sort.
    Fixed(i32),
    /// Index lu dans la valeur d'un objet de l'inventaire (`indexItem`).
    Item(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    pub action: ActionId,
    pub index: OptionIndex,
}

#[derive(Clone, Copy, Debug)]
pub struct Stage {
    pub audio: Option<StrRef>,
    pub image: Option<StrRef>,
    pub ok: Option<Transition>,
    pub home: Option<Transition>,
    pub autoplay: bool,
    /// `control.ok` : le bouton OK passe à la suite pendant la lecture.
    pub ok_enabled: bool,
    pub inventory_reset: bool,
    /// Mises à jour d'inventaire dans [`Story::updates`].
    pub updates: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct Action {
    /// Clé d'origine, gardée pour les sauvegardes compatibles Telmi.
    pub key: StrRef,
    /// Options dans [`Story::options`].
    pub options: Span,
}

#[derive(Clone, Copy, Debug)]
pub struct ActionOption {
    /// `None` quand l'option vise une étape absente du fichier.
    pub stage: Option<StageId>,
    /// Conditions dans [`Story::conditions`].
    pub conditions: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    Number(i32),
    Item(u16),
    PlayingTime,
}

/// Comparaison `item <comparator> operand` (codes 0 à 5 de Telmi Sync).
#[derive(Clone, Copy, Debug)]
pub struct Condition {
    pub item: u16,
    pub comparator: u8,
    pub operand: Operand,
}

/// Opération `item <kind>= operand` (codes 0 à 5 de Telmi Sync).
#[derive(Clone, Copy, Debug)]
pub struct ItemUpdate {
    pub item: u16,
    pub kind: u8,
    pub operand: Operand,
}

#[derive(Clone, Copy, Debug)]
pub struct InventoryItem {
    pub initial: i32,
    pub max: i32,
    /// 0 : compteur, 1 : barre, 2 : caché.
    pub display: u8,
    pub image: Option<StrRef>,
}

/// Histoire compilée, sans aucune référence à l'arbre JSON d'origine.
#[derive(Debug, Default)]
pub struct Story {
    pub(crate) arena: String,
    pub(crate) start: Option<Transition>,
    pub(crate) stages: Vec<Stage>,
    pub(crate) actions: Vec<Option<Action>>,
    pub(crate) options: Vec<ActionOption>,
    pub(crate) conditions: Vec<Condition>,
    pub(crate) updates: Vec<ItemUpdate>,
    pub(crate) inventory: Vec<InventoryItem>,
}

impl Story {
    pub fn str(&self, s: StrRef) -> &str {
        &self.arena[s.start as usize..(s.start + s.len) as usize]
    }

    pub(crate) fn push_str(&mut self, s: &str) -> StrRef {
        let start = self.arena.len() as u32;
        self.arena.push_str(s);
        StrRef { start, len: s.len() as u32 }
    }

    pub fn start(&self) -> Option<Transition> {
        self.start
    }

    pub fn stage(&self, id: StageId) -> Option<&Stage> {
        self.stages.get(id as usize)
    }

    /// `None` si l'action est référencée mais jamais définie.
    pub fn action(&self, id: ActionId) -> Option<&Action> {
        self.actions.get(id as usize).and_then(Option::as_ref)
    }

    pub fn action_by_key(&self, key: &str) -> Option<ActionId> {
        self.actions
            .iter()
            .position(|a| a.is_some_and(|a| self.str(a.key) == key))
            .map(|i| i as ActionId)
    }

    pub fn options(&self, action: &Action) -> &[ActionOption] {
        &self.options[action.options.range()]
    }

    pub fn conditions(&self, option: &ActionOption) -> &[Condition] {
        &self.conditions[option.conditions.range()]
    }

    pub fn updates(&self, stage: &Stage) -> &[ItemUpdate] {
        &self.updates[stage.updates.range()]
    }

    pub fn inventory(&self) -> &[InventoryItem] {
        &self.inventory
    }

    pub fn stage_count(&self) -> usize {
        self.stages.len()
    }

    /// Octets occupés en mémoire par l'histoire (hors en-têtes d'allocation).
    pub fn heap_bytes(&self) -> usize {
        use std::mem::size_of;
        self.arena.capacity()
            + self.stages.capacity() * size_of::<Stage>()
            + self.actions.capacity() * size_of::<Option<Action>>()
            + self.options.capacity() * size_of::<ActionOption>()
            + self.conditions.capacity() * size_of::<Condition>()
            + self.updates.capacity() * size_of::<ItemUpdate>()
            + self.inventory.capacity() * size_of::<InventoryItem>()
    }
}

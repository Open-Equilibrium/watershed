//! ADR-0238 acceptance spike: the same Page operations on Automerge and Loro.
//!
//! A Page document holds `blocks` (Text Blocks with formatted text), `order` (one sort key
//! per View), `rows` (Database rows), `connections` and `automations`. Both libraries
//! implement [`Replica`], so the convergence tests and measurements run unchanged on each.

pub mod am;
pub mod lo;

/// A change a replica received but refused, named by its library's change identity.
pub type Rejected = Vec<String>;

/// The rule every replica applies to received changes in the Role check: `peer` may not
/// change Database rows. It stands in for a Role that allows Text but not the Database.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub denied_rows_peer: u64,
}

pub trait Replica: Sized {
    /// A Page with empty top-level maps, created once and forked by every replica, as a
    /// Page is created on one device.
    fn seed(peer: u64) -> Self;
    fn fork_as(&mut self, peer: u64) -> Self;

    fn add_text_block(&mut self, id: &str, text: &str, sort_key: &str);
    fn edit_text(&mut self, id: &str, pos: usize, delete: usize, insert: &str);
    fn bold(&mut self, id: &str, start: usize, end: usize);
    fn put_cell(&mut self, row: &str, field: &str, value: &str);
    fn move_view(&mut self, id: &str, sort_key: &str);
    fn add_connection(&mut self, id: &str, from: &str, to: &str);
    fn add_automation(&mut self, id: &str, owner: Option<&str>, uses: &str);
    /// Deletes a Block with its View, its Connections and its own Automations, and turns
    /// off Workspace Automations that use it, in one commit.
    fn delete_block(&mut self, id: &str);
    fn commit(&mut self);

    /// Pulls everything `other` has that this replica lacks.
    fn pull(&mut self, other: &mut Self);
    /// Pulls change by change, applying [`Policy`]; returns the refused changes.
    fn pull_checked(&mut self, other: &mut Self, policy: Policy) -> Rejected;

    /// Every value concurrently stored in one Database cell.
    fn cell_values(&mut self, row: &str, field: &str) -> Vec<String>;
    /// Block ids in View order.
    fn order(&mut self) -> Vec<String>;
    fn text(&mut self, id: &str) -> Option<String>;
    /// Every stored Connection as (id, from, to).
    fn connections(&mut self) -> Vec<(String, String, String)>;
    /// Every stored Automation.
    fn automations(&mut self) -> Vec<Automation>;
    /// A canonical rendering of the whole Page, equal on two replicas exactly when their
    /// visible state is equal.
    fn canonical(&mut self) -> String;
    fn save(&mut self) -> Vec<u8>;
    fn load(bytes: &[u8]) -> Self;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Automation {
    pub id: String,
    pub owner: Option<String>,
    pub uses: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutomationState {
    On,
    Off,
    NeedsAttention,
}

/// The read rule after a merge, the same on every replica: a deletion's cascade cannot see
/// what another device creates concurrently, so a Connection whose end names a deleted
/// Block is dangling and hidden.
pub fn dangling_connections<R: Replica>(replica: &mut R) -> Vec<String> {
    let mut dangling: Vec<String> = replica
        .connections()
        .into_iter()
        .filter(|(_, from, to)| replica.text(from).is_none() || replica.text(to).is_none())
        .map(|(id, _, _)| id)
        .collect();
    dangling.sort();
    dangling
}

/// The same read rule for Automations: one whose owning Block is deleted counts as deleted
/// with it and is left out; one that uses a deleted Block needs attention.
pub fn effective_automations<R: Replica>(replica: &mut R) -> Vec<(String, AutomationState)> {
    let mut states = Vec::new();
    for automation in replica.automations() {
        if automation
            .owner
            .as_ref()
            .is_some_and(|owner| replica.text(owner).is_none())
        {
            continue;
        }
        let state = if replica.text(&automation.uses).is_none() {
            AutomationState::NeedsAttention
        } else if automation.enabled {
            AutomationState::On
        } else {
            AutomationState::Off
        };
        states.push((automation.id, state));
    }
    states.sort_by(|x, y| x.0.cmp(&y.0));
    states
}

pub(crate) fn sorted_order(mut keys: Vec<(String, String)>) -> Vec<String> {
    keys.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    keys.into_iter().map(|(id, _)| id).collect()
}

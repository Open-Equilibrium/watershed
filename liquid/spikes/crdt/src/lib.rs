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
    /// A canonical rendering of the whole Page, equal on two replicas exactly when their
    /// visible state is equal.
    fn canonical(&mut self) -> String;
    fn save(&mut self) -> Vec<u8>;
    fn load(bytes: &[u8]) -> Self;
}

/// Connections whose ends name a Block that no longer exists, as a concurrent edit can
/// leave them after a deletion; a deterministic read rule hides them on every replica.
pub fn dangling_connections<R: Replica>(
    replica: &mut R,
    connections: &[(&str, &str, &str)],
) -> Vec<String> {
    connections
        .iter()
        .filter(|(_, from, to)| replica.text(from).is_none() || replica.text(to).is_none())
        .map(|(id, _, _)| (*id).to_string())
        .collect()
}

pub(crate) fn sorted_order(mut keys: Vec<(String, String)>) -> Vec<String> {
    keys.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    keys.into_iter().map(|(id, _)| id).collect()
}

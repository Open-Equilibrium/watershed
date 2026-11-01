//! ADR-0238 acceptance: the same merge cases on Automerge and Loro.

use liquid_crdt_spike::{Policy, Replica, am::Am, dangling_connections, lo::Lo};

fn sync<R: Replica>(a: &mut R, b: &mut R) {
    a.pull(b);
    b.pull(a);
}

fn page<R: Replica>() -> R {
    let mut seed = R::seed(9);
    seed.add_text_block("plan", "Five days in Lisbon", "a1");
    seed.add_text_block("notes", "Tram 28 is crowded", "a2");
    seed.add_text_block("ideas", "Day trip to Sintra", "a3");
    seed.put_cell("rent", "category", "Home");
    seed.add_connection("chart", "plan", "notes");
    seed.add_automation("flag", Some("plan"), "plan");
    seed.add_automation("summary", None, "plan");
    seed.commit();
    seed
}

macro_rules! suite {
    ($name:ident, $replica:ty, $values_kept_on_conflict:expr) => {
        mod $name {
            use super::*;
            type R = $replica;

            #[test]
            fn concurrent_typing_in_one_text_block_keeps_both_edits() {
                let mut seed: R = page();
                let (mut a, mut b) = (seed.fork_as(1), seed.fork_as(2));
                a.edit_text("plan", 0, 0, "Our ");
                b.edit_text("plan", 19, 0, " in May");
                a.commit();
                b.commit();
                sync(&mut a, &mut b);
                assert_eq!(a.canonical(), b.canonical());
                assert_eq!(a.text("plan").unwrap(), "Our Five days in Lisbon in May");
            }

            #[test]
            fn formatting_and_concurrent_typing_converge() {
                let mut seed: R = page();
                let (mut a, mut b) = (seed.fork_as(1), seed.fork_as(2));
                a.bold("plan", 0, 9);
                b.edit_text("plan", 5, 0, "sunny ");
                a.commit();
                b.commit();
                sync(&mut a, &mut b);
                assert_eq!(a.canonical(), b.canonical());
                assert_eq!(a.text("plan").unwrap(), "Five sunny days in Lisbon");
            }

            #[test]
            fn concurrent_edits_of_one_cell_converge() {
                let mut seed: R = page();
                let (mut a, mut b) = (seed.fork_as(1), seed.fork_as(2));
                a.put_cell("rent", "category", "Housing");
                b.put_cell("rent", "category", "Fixed costs");
                a.commit();
                b.commit();
                sync(&mut a, &mut b);
                assert_eq!(a.canonical(), b.canonical());
                assert_eq!(
                    a.cell_values("rent", "category"),
                    b.cell_values("rent", "category")
                );
                assert_eq!(
                    a.cell_values("rent", "category").len(),
                    $values_kept_on_conflict
                );
            }

            #[test]
            fn concurrent_edits_of_different_fields_keep_both() {
                let mut seed: R = page();
                let (mut a, mut b) = (seed.fork_as(1), seed.fork_as(2));
                a.put_cell("rent", "amount", "950.00");
                b.put_cell("rent", "date", "27 Oct");
                a.commit();
                b.commit();
                sync(&mut a, &mut b);
                assert_eq!(a.canonical(), b.canonical());
                assert_eq!(a.cell_values("rent", "amount"), vec!["950.00"]);
                assert_eq!(a.cell_values("rent", "date"), vec!["27 Oct"]);
            }

            #[test]
            fn concurrent_moves_keep_each_view_once() {
                let mut seed: R = page();
                let (mut a, mut b) = (seed.fork_as(1), seed.fork_as(2));
                a.move_view("plan", "a4");
                b.move_view("plan", "a0");
                b.move_view("notes", "a5");
                a.commit();
                b.commit();
                sync(&mut a, &mut b);
                assert_eq!(a.order(), b.order());
                let mut ids = a.order();
                ids.sort();
                assert_eq!(ids, ["ideas", "notes", "plan"]);
            }

            #[test]
            fn deletion_cascade_converges_with_a_concurrent_edit() {
                let mut seed: R = page();
                let (mut a, mut b) = (seed.fork_as(1), seed.fork_as(2));
                a.delete_block("plan");
                b.edit_text("plan", 0, 0, "Our ");
                b.add_connection("late", "notes", "plan");
                b.commit();
                sync(&mut a, &mut b);
                assert_eq!(a.canonical(), b.canonical());
                assert_eq!(a.text("plan"), None);
                assert_eq!(a.order(), ["notes", "ideas"]);
                assert!(
                    !a.canonical().contains("chart:"),
                    "the deletion removed its Connection"
                );
                let late = [("late", "notes", "plan")];
                assert_eq!(dangling_connections(&mut a, &late), ["late"]);
                assert_eq!(dangling_connections(&mut b, &late), ["late"]);
                assert!(!a.canonical().contains("flag:"));
                assert!(a.canonical().contains("enabled:false"));
            }

            #[test]
            fn a_saved_page_loads_unchanged() {
                let mut seed: R = page();
                let mut a = seed.fork_as(1);
                a.bold("notes", 0, 4);
                a.commit();
                let mut loaded = R::load(&a.save());
                assert_eq!(loaded.canonical(), a.canonical());
            }

            #[test]
            fn a_change_a_role_denies_is_refused_identically_on_every_replica() {
                let policy = Policy {
                    denied_rows_peer: 3,
                };
                let mut seed: R = page();
                let mut restricted = seed.fork_as(3);
                restricted.edit_text("notes", 0, 0, "Note: ");
                restricted.commit();
                restricted.put_cell("rent", "category", "Mine");
                restricted.commit();
                restricted.edit_text("ideas", 0, 0, "Maybe: ");
                restricted.commit();

                let (mut a, mut b, mut relay) = (seed.fork_as(1), seed.fork_as(2), seed.fork_as(5));
                let refused_by_a = a.pull_checked(&mut restricted, policy);
                let refused_by_relay = relay.pull_checked(&mut restricted, policy);
                assert!(b.pull_checked(&mut relay, policy).is_empty());
                assert!(a.pull_checked(&mut b, policy).is_empty());
                assert!(b.pull_checked(&mut a, policy).is_empty());

                assert_eq!(refused_by_a, refused_by_relay);
                assert_eq!(
                    refused_by_a.len(),
                    2,
                    "the row change and the change built on it"
                );
                assert_eq!(a.canonical(), b.canonical());
                assert_eq!(a.cell_values("rent", "category"), vec!["Home"]);
                assert_eq!(a.text("notes").unwrap(), "Note: Tram 28 is crowded");
                assert_eq!(a.text("ideas").unwrap(), "Day trip to Sintra");
            }
        }
    };
}

suite!(automerge, Am, 2);
suite!(loro, Lo, 1);

//! The mutation pipeline as its callers see it: every write is checked, applied and
//! recorded as one Action, and a refused write changes nothing.

use liquid_core::{
    Actor, Applied, AreaId, Error, Mutation, Operation, Origin, PageId, Permission, PersonId,
    Request, ViewId, Workspace,
};

fn owner() -> Actor {
    Actor::Person(PersonId::new("benni"))
}

fn apply(workspace: &mut Workspace, mutation: Mutation) -> Applied {
    workspace
        .apply(Request {
            actor: owner(),
            origin: Origin::Ui,
            mutation,
        })
        .expect("the owner may apply it")
}

fn area(workspace: &mut Workspace, name: &str) -> AreaId {
    match apply(workspace, Mutation::CreateArea { name: name.into() }).created {
        Some(liquid_core::Created::Area(id)) => id,
        other => panic!("expected an area, got {other:?}"),
    }
}

fn page(workspace: &mut Workspace, area: &AreaId, parent: Option<&PageId>, title: &str) -> PageId {
    let mutation = Mutation::CreatePage {
        area: area.clone(),
        parent: parent.cloned(),
        title: title.into(),
    };
    match apply(workspace, mutation).created {
        Some(liquid_core::Created::Page(id)) => id,
        other => panic!("expected a Page, got {other:?}"),
    }
}

fn text_block(
    workspace: &mut Workspace,
    page: &PageId,
    after: Option<&ViewId>,
    text: &str,
) -> (liquid_core::BlockId, ViewId) {
    let mutation = Mutation::AddTextBlock {
        page: page.clone(),
        after: after.cloned(),
        text: text.into(),
    };
    match apply(workspace, mutation).created {
        Some(liquid_core::Created::Block { block, view }) => (block, view),
        other => panic!("expected a Block, got {other:?}"),
    }
}

fn titles(workspace: &Workspace, area: &AreaId) -> Vec<(String, Option<String>)> {
    let title_of = |id: &PageId| {
        workspace
            .pages(area)
            .into_iter()
            .find(|entry| &entry.id == id)
            .map(|entry| entry.title)
    };
    workspace
        .pages(area)
        .iter()
        .map(|entry| {
            (
                entry.title.clone(),
                entry.parent.as_ref().and_then(title_of),
            )
        })
        .collect()
}

fn texts(workspace: &Workspace, page: &PageId) -> Vec<String> {
    workspace
        .views(page)
        .iter()
        .map(|view| workspace.text(page, &view.block).expect("a live Block"))
        .collect()
}

#[test]
fn the_explorer_lists_areas_and_nested_pages_in_creation_order() {
    let mut workspace = Workspace::create(PersonId::new("benni")).unwrap();
    let trips = area(&mut workspace, "Trips");
    let home = area(&mut workspace, "Home");
    let lisbon = page(&mut workspace, &trips, None, "Lisbon");
    page(&mut workspace, &trips, None, "Porto");
    page(&mut workspace, &trips, Some(&lisbon), "Day trips");
    page(&mut workspace, &home, None, "Budget");

    let names: Vec<_> = workspace
        .areas()
        .into_iter()
        .map(|area| area.name)
        .collect();
    assert_eq!(names, ["Trips", "Home"]);
    assert_eq!(
        titles(&workspace, &trips),
        [
            ("Lisbon".into(), None),
            ("Day trips".into(), Some("Lisbon".into())),
            ("Porto".into(), None),
        ]
    );

    apply(
        &mut workspace,
        Mutation::RenamePage {
            page: lisbon.clone(),
            title: "Lisbon in May".into(),
        },
    );
    assert_eq!(titles(&workspace, &trips)[0].0, "Lisbon in May");
}

#[test]
fn text_blocks_keep_their_order_and_edit_in_utf16_units() {
    let mut workspace = Workspace::create(PersonId::new("benni")).unwrap();
    let trips = area(&mut workspace, "Trips");
    let lisbon = page(&mut workspace, &trips, None, "Lisbon");
    let (plan, plan_view) = text_block(&mut workspace, &lisbon, None, "Five days");
    let (_, notes_view) = text_block(&mut workspace, &lisbon, Some(&plan_view), "Tram 28");
    text_block(&mut workspace, &lisbon, Some(&plan_view), "Sintra 🌄");
    text_block(&mut workspace, &lisbon, Some(&notes_view), "Pastéis");
    assert_eq!(
        texts(&workspace, &lisbon),
        ["Five days", "Sintra 🌄", "Tram 28", "Pastéis"]
    );

    // "🌄" is two UTF-16 units, as Dart and JavaScript count text.
    let sintra = workspace.views(&lisbon)[1].block.clone();
    apply(
        &mut workspace,
        Mutation::EditText {
            page: lisbon.clone(),
            block: sintra.clone(),
            at: 9,
            delete: 0,
            insert: "!".into(),
        },
    );
    apply(
        &mut workspace,
        Mutation::EditText {
            page: lisbon.clone(),
            block: plan,
            at: 0,
            delete: 4,
            insert: "Six".into(),
        },
    );
    assert_eq!(texts(&workspace, &lisbon)[..2], ["Six days", "Sintra 🌄!"]);

    for (at, delete) in [(8, 0), (7, 1), (10, 1)] {
        let refused = workspace.apply(Request {
            actor: owner(),
            origin: Origin::Ui,
            mutation: Mutation::EditText {
                page: lisbon.clone(),
                block: sintra.clone(),
                at,
                delete,
                insert: "x".into(),
            },
        });
        assert_eq!(
            refused.unwrap_err(),
            Error::OutOfRange,
            "at {at}, delete {delete}"
        );
    }
    assert_eq!(texts(&workspace, &lisbon)[1], "Sintra 🌄!");
}

#[test]
fn removing_the_last_view_removes_its_block() {
    let mut workspace = Workspace::create(PersonId::new("benni")).unwrap();
    let trips = area(&mut workspace, "Trips");
    let lisbon = page(&mut workspace, &trips, None, "Lisbon");
    let (plan, plan_view) = text_block(&mut workspace, &lisbon, None, "Five days");
    text_block(&mut workspace, &lisbon, Some(&plan_view), "Tram 28");

    let applied = apply(
        &mut workspace,
        Mutation::RemoveView {
            page: lisbon.clone(),
            view: plan_view.clone(),
        },
    );
    assert_eq!(applied.created, None);
    assert_eq!(texts(&workspace, &lisbon), ["Tram 28"]);
    assert_eq!(workspace.text(&lisbon, &plan), None);

    for mutation in [
        Mutation::RemoveView {
            page: lisbon.clone(),
            view: plan_view.clone(),
        },
        Mutation::AddTextBlock {
            page: lisbon.clone(),
            after: Some(plan_view.clone()),
            text: "After a removed View".into(),
        },
        Mutation::EditText {
            page: lisbon.clone(),
            block: plan,
            at: 0,
            delete: 0,
            insert: "Our ".into(),
        },
    ] {
        let refused = workspace.apply(Request {
            actor: owner(),
            origin: Origin::Ui,
            mutation,
        });
        assert_eq!(refused.unwrap_err(), Error::NotFound);
    }
}

#[test]
fn a_refused_request_changes_nothing_and_records_no_action() {
    let mut workspace = Workspace::create(PersonId::new("benni")).unwrap();
    let trips = area(&mut workspace, "Trips");
    let home = area(&mut workspace, "Home");
    let budget = page(&mut workspace, &home, None, "Budget");
    let before = workspace.history().len();

    let stranger = workspace.apply(Request {
        actor: Actor::Person(PersonId::new("mallory")),
        origin: Origin::Ui,
        mutation: Mutation::CreatePage {
            area: trips.clone(),
            parent: None,
            title: "Mine".into(),
        },
    });
    assert_eq!(stranger.unwrap_err(), Error::NotAllowed);

    let other_area_parent = workspace.apply(Request {
        actor: owner(),
        origin: Origin::Ui,
        mutation: Mutation::CreatePage {
            area: trips.clone(),
            parent: Some(budget),
            title: "Misplaced".into(),
        },
    });
    assert_eq!(other_area_parent.unwrap_err(), Error::NotFound);

    assert!(workspace.pages(&trips).is_empty());
    assert_eq!(workspace.history().len(), before);
}

#[test]
fn every_action_names_its_actor_origin_and_permission() {
    let mut workspace = Workspace::create(PersonId::new("benni")).unwrap();
    let trips = area(&mut workspace, "Trips");
    let lisbon = page(&mut workspace, &trips, None, "Lisbon");
    let applied = workspace
        .apply(Request {
            actor: owner(),
            origin: Origin::Api,
            mutation: Mutation::AddTextBlock {
                page: lisbon.clone(),
                after: None,
                text: "Five days".into(),
            },
        })
        .unwrap();

    let history = workspace.history();
    assert_eq!(history.len(), 4);
    assert_eq!(history[0].operation, Operation::CreateWorkspace);
    assert!(matches!(
        history[1].operation,
        Operation::Mutation(Mutation::CreateArea { .. })
    ));
    let last = history.last().unwrap();
    assert_eq!(last.id, applied.action);
    assert_eq!(last.actor, owner());
    assert_eq!(last.origin, Origin::Api);
    assert_eq!(last.permission, Permission::Owner);
    assert!(history.iter().all(|action| action.actor == owner()));
}

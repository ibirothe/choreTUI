use std::{fs, process::Command};

use choretui::{
    app::{
        editor::{ChoreSubmission, SchedulePattern},
        use_cases::UseCases,
    },
    config::{CONFIG_ENV, DATA_DIR_ENV},
    domain::{
        CalendarDate, ChoreName, IsoWeekday, Occurrence, OccurrenceState, RecurrenceInterval,
        Timestamp, ports::Clock,
    },
    storage::SqliteStore,
    tui::{
        BoardRuntime,
        model::{BoardInput, BoardLayout},
        screens::catalog::PlanningStatus,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::tempdir_in;

#[derive(Clone, Copy)]
struct FixedClock {
    today: CalendarDate,
    now: Timestamp,
}

impl Clock for FixedClock {
    fn today(&self) -> CalendarDate {
        self.today
    }

    fn now(&self) -> Timestamp {
        self.now
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn send_text(board: &mut BoardRuntime<UseCases<SqliteStore, FixedClock>>, text: &str) {
    for character in text.chars() {
        board.handle_key(key(KeyCode::Char(character)), BoardLayout::SevenColumns);
    }
}

#[test]
fn catalog_selection_customization_and_provenance_survive_restart() {
    let temporary = tempdir_in(std::env::current_dir().expect("working directory should exist"))
        .expect("temporary data directory should be created");
    let database = temporary.path().join("catalog-workflow.db");
    let today = CalendarDate::new(2026, 9, 7).expect("date should be valid");
    let clock = FixedClock {
        today,
        now: Timestamp::from_unix_timestamp(100).expect("timestamp should be valid"),
    };
    let mut board = BoardRuntime::new(UseCases::new(
        SqliteStore::open(&database).expect("database should open"),
        clock,
    ));

    board.handle_input(BoardInput::AddChore, BoardLayout::SevenColumns);
    board.handle_key(key(KeyCode::F(2)), BoardLayout::SevenColumns);
    board.handle_key(key(KeyCode::Char('/')), BoardLayout::SevenColumns);
    send_text(&mut board, "wipe bathroom fixtures");
    board.handle_key(key(KeyCode::Enter), BoardLayout::SevenColumns);
    assert_eq!(
        board
            .catalog_browser()
            .and_then(|catalog| catalog.selected_activity())
            .map(|activity| activity.id.as_str()),
        Some("bathroom.wipe_fixtures")
    );

    board.handle_key(key(KeyCode::Enter), BoardLayout::SevenColumns);
    assert!(board.editor().is_some());
    send_text(&mut board, "-upstairs");
    board.handle_key(
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        BoardLayout::SevenColumns,
    );
    assert!(board.editor().is_none());
    assert_eq!(
        board
            .catalog_browser()
            .map(|catalog| catalog.planning_status("bathroom.wipe_fixtures")),
        Some(PlanningStatus::Planned)
    );
    assert_eq!(
        board
            .state()
            .selected_occurrence()
            .map(|occurrence| occurrence.name().as_str()),
        Some("Wipe bathroom fixtures-upstairs")
    );

    let (application, _) = board.into_parts();
    let (store, _) = application.into_parts();
    drop(store);
    let restarted = BoardRuntime::new(UseCases::new(
        SqliteStore::open(&database).expect("database should reopen"),
        FixedClock {
            today,
            now: Timestamp::from_unix_timestamp(200).expect("timestamp should be valid"),
        },
    ));
    assert_eq!(
        restarted
            .state()
            .selected_occurrence()
            .map(|occurrence| occurrence.name().as_str()),
        Some("Wipe bathroom fixtures-upstairs")
    );
    let (application, _) = restarted.into_parts();
    let planning = application
        .catalog_planning()
        .expect("planning data should load after restart");
    assert_eq!(planning.len(), 1);
    assert_eq!(planning[0].name, "Wipe bathroom fixtures-upstairs");
    assert_eq!(
        planning[0]
            .provenance
            .as_ref()
            .map(|value| (value.template_id.as_str(), value.catalog_version)),
        Some(("bathroom.wipe_fixtures", 2))
    );
}

#[test]
fn primary_workflow_survives_restart_and_passes_doctor() {
    let temporary = tempdir_in(std::env::current_dir().expect("working directory should exist"))
        .expect("temporary XDG tree should be created");
    let data_dir = temporary.path().join("data");
    fs::create_dir_all(&data_dir).expect("data directory should be created");
    let database = data_dir.join("choretui.db");
    let config = temporary.path().join("config.toml");
    let today = CalendarDate::new(2026, 9, 7).expect("date should be valid");
    let clock = FixedClock {
        today,
        now: Timestamp::from_unix_timestamp(100).expect("timestamp should be valid"),
    };

    let store = SqliteStore::open(&database).expect("database should open");
    let mut application = UseCases::new(store, clock);
    application
        .save_editor(ChoreSubmission {
            id: None,
            name: ChoreName::new("End-to-end bins").expect("name should be valid"),
            description: None,
            enabled: true,
            pattern: SchedulePattern::Weekly {
                interval: RecurrenceInterval::new(1).expect("interval should be valid"),
                weekdays: vec![IsoWeekday::Monday],
            },
            provenance: None,
        })
        .expect("chore should save");
    let mut board = BoardRuntime::new(application);
    assert!(board.state().selected_occurrence().is_some());
    board.handle_input(BoardInput::ToggleCompletion, BoardLayout::SevenColumns);
    assert!(matches!(
        board.state().selected_occurrence().map(Occurrence::state),
        Some(OccurrenceState::Completed { .. })
    ));
    let (application, _) = board.into_parts();
    let (store, _) = application.into_parts();
    drop(store);

    let reopened = SqliteStore::open(&database).expect("database should reopen");
    let restarted = BoardRuntime::new(UseCases::new(
        reopened,
        FixedClock {
            today,
            now: Timestamp::from_unix_timestamp(200).expect("timestamp should be valid"),
        },
    ));
    assert!(matches!(
        restarted
            .state()
            .selected_occurrence()
            .map(Occurrence::state),
        Some(OccurrenceState::Completed { .. })
    ));
    drop(restarted);

    let output = Command::new(env!("CARGO_BIN_EXE_chore"))
        .arg("doctor")
        .env(CONFIG_ENV, &config)
        .env(DATA_DIR_ENV, &data_dir)
        .output()
        .expect("doctor should start");
    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("integrity"));
    assert!(report.contains("PASS"));
}

use ct::app::App;
use ct::event::AppEvent;
use ct::frame::collect;
use ct::pane::Notify;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

#[test]
fn a_burst_of_output_is_handled_before_one_draw() {
    let (tx, rx) = channel();
    let mut app = App::for_test(vec![]);
    for i in 0..5 {
        tx.send(AppEvent::Pty(Notify::Output(i))).unwrap();
    }
    // a straggler lands inside the settle window
    let t2 = tx.clone();
    let h = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2));
        t2.send(AppEvent::Pty(Notify::Output(9))).unwrap();
    });
    assert!(collect(&mut app, &rx));
    h.join().unwrap();
    // all of it was consumed by that one call
    assert!(rx.try_recv().is_err());
}

#[test]
fn a_steady_stream_cannot_delay_the_frame_past_the_cap() {
    let (tx, rx) = channel();
    let mut app = App::for_test(vec![]);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s2 = stop.clone();
    let h = std::thread::spawn(move || {
        while !s2.load(std::sync::atomic::Ordering::SeqCst) {
            if tx.send(AppEvent::Pty(Notify::Output(1))).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let t = Instant::now();
    assert!(collect(&mut app, &rx));
    let took = t.elapsed();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    h.join().unwrap();
    assert!(took < Duration::from_millis(60), "{took:?}");
}

#[test]
fn quit_stops_collecting() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let (tx, rx) = channel();
    let mut app = App::for_test(vec![]);
    let k = |c, m| AppEvent::Input(Event::Key(KeyEvent::new(c, m)));
    tx.send(k(KeyCode::Char(' '), KeyModifiers::CONTROL)).unwrap();
    tx.send(k(KeyCode::Char('q'), KeyModifiers::NONE)).unwrap();
    assert!(!collect(&mut app, &rx));
}

#[test]
fn quit_event_from_a_signal_stops_the_loop() {
    let (tx, rx) = channel();
    let mut app = App::for_test(vec![]);
    tx.send(AppEvent::Quit).unwrap();
    assert!(!collect(&mut app, &rx));
}

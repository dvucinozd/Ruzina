#[allow(dead_code)]
mod support;
use pajoniiir_media_fs::{
    AsyncFileSystem,
    analysis::{Client, Error},
};
use std::{
    future::Future,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};
use support::broker::*;
use support::*;
fn cancel(f: impl Future) {
    assert!(matches!(
        pin!(f)
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
}
#[test]
fn owned_completion_recovery_and_repeated_jobs() {
    let media = mounted();
    let ctl = Rc::new(Control::default());
    let fs = Fs::new(&media, wav(300));
    let mut client = Client::new(
        Broker::new(fs, ctl.clone()),
        media.lease().unwrap(),
        Box::leak(Box::new([0; 4096])),
    );
    // Cancel open after owner has produced the owned handle.
    ctl.pause.set(true);
    cancel(client.open_read("/TRACK.WAV"));
    assert!(client.pending_ticket().is_some());
    assert!(matches!(
        run(client.open_read("/TRACK.WAV")),
        Err(Error::Busy)
    ));
    ctl.pause.set(false);
    run(client.recover()).unwrap();
    assert_eq!(ctl.closes.get(), 1);
    for _ in 0..3 {
        let mut file = run(client.open_read("/TRACK.WAV")).unwrap();
        let mut bytes = [0; 7];
        assert_eq!(run(client.read(&mut file, &mut bytes)).unwrap(), 7);
        ctl.pause.set(true);
        cancel(client.read(&mut file, &mut bytes));
        let pending = client.pending_ticket();
        ctl.pause.set(false);
        ctl.unexpected.set(true);
        assert!(matches!(run(client.recover()), Err(Error::Unexpected)));
        assert_eq!(pending, client.pending_ticket());
        assert!(client.take_unexpected().is_some());
        run(client.close_file(file)).unwrap();
        assert!(client.is_idle());
    }
    // Queue refusal returns both original handle and whole buffer.
    let mut file = run(client.open_read("/TRACK.WAV")).unwrap();
    ctl.full.set(true);
    assert!(matches!(
        run(client.read(&mut file, &mut [0; 5])),
        Err(Error::QueueFull)
    ));
    ctl.full.set(false);
    run(client.close_file(file)).unwrap();
    // Stale lease after dispatch withholds bytes and still recovers ownership.
    let mut file = run(client.open_read("/TRACK.WAV")).unwrap();
    ctl.pause.set(true);
    cancel(client.read(&mut file, &mut [0; 5]));
    ctl.stale.set(true);
    ctl.pause.set(false);
    assert!(matches!(run(client.close_file(file)), Err(Error::Stale)));
    assert!(client.is_idle());
    assert!(matches!(
        run(client.open_read("/TRACK.WAV")),
        Err(Error::Stale)
    ));
    ctl.stale.set(false);
    let file = run(client.open_read("/TRACK.WAV")).unwrap();
    ctl.pause.set(true);
    cancel(client.close_file(file));
    ctl.pause.set(false);
    run(client.recover()).unwrap();
    assert!(client.is_idle());
    assert_eq!(ctl.closes.get(), 7);
    free_buffer(client.retire_buffer().unwrap());
}

#[test]
fn errors_stale_completion_and_replacement_preserve_recovery() {
    for mode in [1, 3, 5, 6, 7, 11, 12, 13] {
        let media = mounted();
        let ctl = Rc::new(Control::default());
        let mut fs = Fs::new(&media, wav(300));
        fs.mode = mode;
        let mut client = Client::new(
            Broker::new(fs, ctl.clone()),
            media.lease().unwrap(),
            Box::leak(Box::new([0; 4096])),
        );
        let opened = run(client.open_read("/TRACK.WAV"));
        if mode == 11 {
            assert!(matches!(opened, Err(Error::Stale)));
        } else {
            let mut file = opened.unwrap();
            let mut out = [0xa5; 17];
            let result = run(client.read(&mut file, &mut out));
            if mode != 5 && mode != 13 {
                assert!(result.is_err());
                assert_eq!(out, [0xa5; 17]);
            }
            let closed = run(client.close_file(file));
            if [5, 12, 13].contains(&mode) {
                assert!(closed.is_err());
            } else {
                closed.unwrap();
            }
        }
        // Backend error is surfaced once, recovered buffer remains exclusive.
        let _ = run(client.recover());
        run(client.recover()).unwrap();
        assert!(client.is_idle());
        assert_eq!(ctl.closes.get(), 1);
        free_buffer(client.retire_buffer().unwrap());
    }
    let media = mounted();
    let ctl = Rc::new(Control::default());
    let mut client = Client::new(
        Broker::new(Fs::new(&media, wav(300)), ctl.clone()),
        media.lease().unwrap(),
        Box::leak(Box::new([0; 4096])),
    );
    let mut old = run(client.open_read("/TRACK.WAV")).unwrap();
    ctl.wrong_ticket.set(true);
    assert!(matches!(
        run(client.read(&mut old, &mut [0; 5])),
        Err(Error::Unexpected)
    ));
    assert!(client.pending_ticket().is_some());
    assert!(client.take_unexpected().is_some());
    run(client.recover()).unwrap();
    let mut new = run(client.open_read("/TRACK.WAV")).unwrap();
    assert!(matches!(run(client.close_file(old)), Err(Error::Contract)));
    ctl.stale_on_receive.set(true);
    let mut out = [0xa5; 5];
    assert!(matches!(
        run(client.read(&mut new, &mut out)),
        Err(Error::Stale)
    ));
    assert_eq!(out, [0xa5; 5]);
    run(client.recover()).unwrap();
    ctl.stale.set(false);
    run(client.close_file(new)).unwrap();
    free_buffer(client.retire_buffer().unwrap());
}

#[test]
fn replaced_scan_drains_broker_before_next_job() {
    use pajoniiir_apta_adapter::{
        source::SourceRequest, streaming::StreamLimits, worker::Requests,
    };
    let media = mounted();
    let catalog = catalog(&media);
    let mut requests = Requests::default();
    let ctl = Rc::new(Control::default());
    ctl.yield_receive.set(true);
    let mut fs = Client::new(
        Broker::new(Fs::new(&media, wav(300)), ctl.clone()),
        media.lease().unwrap(),
        Box::leak(Box::new([0; 4096])),
    );
    let mut bytes = [0; 4096];
    let mut scan = None;
    let request = SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
    {
        let mut f = pin!(request.scan(
            &mut scan,
            &mut fs,
            &mut requests,
            &media,
            StreamLimits::UNLIMITED,
            &mut bytes
        ));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(f.as_mut().poll(&mut cx).is_pending()); // open
        assert!(f.as_mut().poll(&mut cx).is_pending()); // read
    }
    assert!(scan.is_some());
    assert!(fs.pending_ticket().is_some());
    let replacement = SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
    assert!(
        drive(replacement.scan(
            &mut scan,
            &mut fs,
            &mut requests,
            &media,
            StreamLimits::UNLIMITED,
            &mut bytes
        ))
        .is_err()
    );
    drive(scan.take().unwrap().abort(&mut fs)).unwrap();
    assert!(fs.is_idle());
    let next = SourceRequest::new(&mut requests, &media, &catalog, 0).unwrap();
    drive(next.scan(
        &mut scan,
        &mut fs,
        &mut requests,
        &media,
        StreamLimits::UNLIMITED,
        &mut bytes,
    ))
    .unwrap();
    assert!(fs.is_idle());
    assert_eq!(ctl.closes.get(), 2);
    free_buffer(fs.retire_buffer().unwrap());
}

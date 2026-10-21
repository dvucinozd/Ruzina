use super::*;
use pajoniiir_controller_core::{BeatFxTarget, ControlEvent, DeckId};
use pajoniiir_deck::{BeatFxBeat, BeatFxEffect, PadFxBank};
use pajoniiir_ui_model::{
    BeatFxAction, MixerParameter, MixerToggle, fx_depth_event, fx_target_event, pad_fx_event,
    present_fx, present_mixer,
};

fn target(code: i32) -> Option<Option<DeckId>> {
    match code {
        -1 => Some(None),
        0 => Some(Some(DeckId::One)),
        1 => Some(Some(DeckId::Two)),
        _ => None,
    }
}
fn apply(ui: &slint::Weak<MainWindow>, state: &RefCell<Session>, event: Option<ControlEvent>) {
    if let Some(event) = event {
        state.borrow_mut().control(event);
    }
    if let Some(ui) = ui.upgrade() {
        super::refresh(&ui, &state.borrow());
    }
}
pub fn bind(ui: &MainWindow, state: Rc<RefCell<Session>>) {
    ui.set_channels(ModelRc::new(VecModel::from(vec![
        MixerChannelView::default(
        );
        2
    ])));
    ui.set_pads(ModelRc::new(VecModel::from(vec![PadFxView::default(); 2])));
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_mixer_level(move |i, code, raw| {
        let event = target(i).and_then(|deck| MixerParameter::from_code(code)?.event(deck, raw));
        apply(&weak, &s, event);
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_mixer_toggle(move |i, code| {
        let event = target(i).and_then(|deck| MixerToggle::from_code(code)?.event(deck, true));
        apply(&weak, &s, event);
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_fx_action(move |code| {
        apply(
            &weak,
            &s,
            BeatFxAction::from_code(code).map(|a| a.event(true)),
        );
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_fx_target(move |code| {
        apply(&weak, &s, fx_target_event(code));
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_fx_depth(move |raw| {
        apply(&weak, &s, fx_depth_event(raw));
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_pad_bank(move |i, bank| {
        if (0..2).contains(&i) {
            s.borrow_mut().select_pad_bank(deck(i as usize), bank);
        }
        if let Some(ui) = weak.upgrade() {
            super::refresh(&ui, &s.borrow());
        }
    });
    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_pad_fx(move |i, bank, pad, pressed| {
        if (0..2).contains(&i) {
            apply(
                &weak,
                &s,
                pad_fx_event(deck(i as usize), bank, pad, pressed),
            );
        }
    });
    let weak = ui.as_weak();
    ui.on_cancel_pads(move || {
        state.borrow_mut().cancel_pads();
        if let Some(ui) = weak.upgrade() {
            super::refresh(&ui, &state.borrow());
        }
    });
}

pub fn refresh(ui: &MainWindow, session: &Session) {
    let p = present_mixer(&session.mixer);
    let model = ui.get_channels();
    let rows = model
        .as_any()
        .downcast_ref::<VecModel<MixerChannelView>>()
        .expect("bound mixer model");
    for (i, c) in p.channels.into_iter().enumerate() {
        rows.set_row_data(
            i,
            MixerChannelView {
                volume: i32::from(c.channel_volume),
                trim: i32::from(c.trim),
                high: i32::from(c.eq_high),
                mid: i32::from(c.eq_mid),
                low: i32::from(c.eq_low),
                filter: i32::from(c.filter),
                pfl: c.pfl_enabled,
            },
        );
    }
    ui.set_mixer(MixerView {
        crossfader: i32::from(p.crossfader),
        master: i32::from(p.master_volume),
        headphone_mix: i32::from(p.headphone_mix),
        headphone_level: i32::from(p.headphone_level),
        master_cue: p.master_cue,
        smart_cfx: p.smart_cfx,
        smart_fader: p.smart_fader,
    });
    let fx = present_fx(&session.decks);
    let effect = match fx.beat.effect {
        BeatFxEffect::Filter => "FILTER",
        BeatFxEffect::Echo => "ECHO",
        BeatFxEffect::Flanger => "FLANGER",
        BeatFxEffect::Delay => "DELAY",
    };
    let beat = match fx.beat.beat {
        BeatFxBeat::Quarter => "1/4",
        BeatFxBeat::Half => "1/2",
        BeatFxBeat::One => "1",
        BeatFxBeat::Two => "2",
        BeatFxBeat::Four => "4",
    };
    ui.set_fx(FxView {
        effect: effect.into(),
        beat: beat.into(),
        target: match fx.beat.target {
            BeatFxTarget::ChannelOne => 0,
            BeatFxTarget::ChannelTwo => 1,
            BeatFxTarget::Both => 2,
        },
        depth: i32::from(fx.beat.depth),
        enabled: fx.beat.enabled,
    });
    let model = ui.get_pads();
    let rows = model
        .as_any()
        .downcast_ref::<VecModel<PadFxView>>()
        .expect("bound pad model");
    for (i, pad) in fx.pads.into_iter().enumerate() {
        rows.set_row_data(
            i,
            PadFxView {
                bank: if pad.bank == PadFxBank::One { 0 } else { 1 },
                active_bank: pad
                    .active
                    .map(|p| if p.bank == PadFxBank::One { 0 } else { 1 })
                    .unwrap_or(-1),
                active_pad: pad.active.map(|p| i32::from(p.pad)).unwrap_or(-1),
                loaded: session.presentation(i).transport.loaded,
            },
        );
    }
}

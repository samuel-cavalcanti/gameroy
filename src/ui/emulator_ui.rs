use crate::{
    event_table::EventTable,
    layout::PixelPerfectLayout,
    split_view::SplitView,
    style::Style,
    ui::{Textures, Ui},
    AppState, EmulatorEvent, UserEvent,
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{mpsc::SyncSender, Arc},
};

use crui::{
    graphics::Texture,
    layouts::{FitText, HBoxLayout, MarginLayout, VBoxLayout},
    text::Text,
    widgets::{ButtonGroup, OnKeyboardEvent, TabButton},
    BuilderContext, Gui,
};
use gameroy::{debugger::Debugger, gameboy::GameBoy};
use parking_lot::Mutex;

use winit::event_loop::EventLoopProxy;
mod disassembler_viewer;
mod ppu_viewer;

pub fn create_emulator_ui(
    ui: &mut Ui,
    gb: Arc<parking_lot::lock_api::Mutex<parking_lot::RawMutex, GameBoy>>,
    debugger: Arc<parking_lot::lock_api::Mutex<parking_lot::RawMutex, Debugger>>,
    emu_channel: SyncSender<EmulatorEvent>,
    app_state: AppState,
) {
    ui.gui.set::<Arc<Mutex<GameBoy>>>(gb);
    ui.gui.set::<Arc<Mutex<Debugger>>>(debugger);
    ui.gui.set(emu_channel);
    let debug = app_state.debug;
    ui.gui.set(app_state);

    create_gui(
        &mut ui.gui,
        &ui.textures,
        ui.event_table.clone(),
        &ui.style,
        debug,
    );
}

pub fn create_gui(
    gui: &mut Gui,
    textures: &Textures,
    event_table: Rc<RefCell<EventTable>>,
    style: &Style,
    debug: bool,
) {
    let root = gui.reserve_id();
    let mut screen_id = gui.reserve_id();
    let mut split_view = gui.reserve_id();

    let sty = style.clone();
    let event_table_clone = event_table.clone();
    gui.create_control_reserved(root)
        .behaviour(OnKeyboardEvent::new(move |event, _, ctx| {
            use crui::KeyboardEvent::*;
            use winit::event::VirtualKeyCode::*;
            let sender = ctx.get::<SyncSender<EmulatorEvent>>().clone();
            let debug = ctx.get::<crate::AppState>().debug;
            let app_state = ctx.get_mut::<crate::AppState>();
            let mut set_key = |key: u8, value: bool| {
                app_state.joypad = (app_state.joypad & !(1 << key)) | ((!value as u8) << key)
            };
            match event {
                Pressed(Right) => set_key(0, true), // Left
                Release(Right) => set_key(0, false),
                Pressed(Left) => set_key(1, true), // Right
                Release(Left) => set_key(1, false),
                Pressed(Up) => set_key(2, true), // Up
                Release(Up) => set_key(2, false),
                Pressed(Down) => set_key(3, true), // Down
                Release(Down) => set_key(3, false),
                Pressed(A) => set_key(4, true), // A
                Release(A) => set_key(4, false),
                Pressed(S) => set_key(5, true), // B
                Release(S) => set_key(5, false),
                Pressed(Back) => set_key(6, true), // Select
                Release(Back) => set_key(6, false),
                Pressed(Return) => set_key(7, true), // Start
                Release(Return) => set_key(7, false),
                event => {
                    if debug {
                        match event {
                            Pressed(F5) => {
                                sender.send(EmulatorEvent::SaveState).unwrap();
                            }
                            Pressed(F6) => {
                                sender.send(EmulatorEvent::LoadState).unwrap();
                            }
                            Pressed(F7) => {
                                sender.send(EmulatorEvent::StepBack).unwrap();
                            }
                            Pressed(F8) => {
                                sender.send(EmulatorEvent::Step).unwrap();
                            }
                            Pressed(F9) => {
                                sender.send(EmulatorEvent::Run).unwrap();
                            }
                            Pressed(F12) => {
                                let textures = ctx.get::<Textures>().clone();
                                close_debug_panel(
                                    ctx,
                                    &textures,
                                    &mut split_view,
                                    &mut screen_id,
                                    root,
                                    &sty,
                                );
                            }
                            _ => {}
                        }
                    } else {
                        match event {
                            Pressed(F5) => {
                                sender.send(EmulatorEvent::SaveState).unwrap();
                            }
                            Pressed(F6) => {
                                sender.send(EmulatorEvent::LoadState).unwrap();
                            }
                            Pressed(F12) => {
                                let textures = ctx.get::<Textures>().clone();
                                // Debug
                                open_debug_panel(
                                    ctx,
                                    &textures,
                                    split_view,
                                    root,
                                    &sty,
                                    &mut screen_id,
                                    event_table.clone(),
                                );
                            }
                            Pressed(LShift) | Release(LShift) => sender
                                .send(EmulatorEvent::FrameLimit(!matches!(event, Pressed(_))))
                                .unwrap(),
                            Pressed(R) | Release(R) => sender
                                .send(EmulatorEvent::Rewind(matches!(event, Pressed(_))))
                                .unwrap(),

                            _ => {}
                        }
                    }
                }
            }
            true
        }))
        .build(gui);

    if debug {
        open_debug_panel(
            &mut gui.get_context(),
            textures,
            split_view,
            root,
            style,
            &mut screen_id,
            event_table_clone,
        );
    } else {
        gui.create_control_reserved(screen_id)
            .parent(root)
            .graphic(style.background.clone())
            .layout(PixelPerfectLayout::new((160, 144), (0, 0)))
            .child(gui, |cb, _| {
                cb.graphic(Texture::new(textures.screen, [0.0, 0.0, 1.0, 1.0]))
            })
            .build(gui);
        gui.set_focus(Some(screen_id));
    }
}

fn close_debug_panel(
    ctx: &mut crui::Context,
    textures: &Textures,
    split_view: &mut crui::Id,
    screen_id: &mut crui::Id,
    root: crui::Id,
    style: &Style,
) {
    ctx.remove(*split_view);
    *split_view = ctx.reserve();
    *screen_id = ctx.reserve();
    ctx.create_control_reserved(*screen_id)
        .parent(root)
        .graphic(style.background.clone())
        .layout(PixelPerfectLayout::new((160, 144), (0, 0)))
        .child(ctx, |cb, _| {
            cb.graphic(Texture::new(textures.screen, [0.0, 0.0, 1.0, 1.0]))
        })
        .build(ctx);
    ctx.set_focus(*screen_id);
    let proxy = ctx.get::<EventLoopProxy<UserEvent>>();
    proxy.send_event(UserEvent::Debug(false)).unwrap();
}

fn open_debug_panel(
    ctx: &mut crui::Context,
    textures: &Textures,
    split_view: crui::Id,
    root: crui::Id,
    style: &Style,
    screen_id: &mut crui::Id,
    event_table: Rc<RefCell<EventTable>>,
) {
    ctx.create_control_reserved(split_view)
        .parent(root)
        .graphic(style.split_background.clone())
        .behaviour_and_layout(SplitView::new(0.333, 4.0, [2.0; 4], false))
        .build(ctx);
    ctx.remove(*screen_id);

    // create screen
    *screen_id = ctx.reserve();
    ctx.create_control_reserved(*screen_id)
        .parent(split_view)
        .graphic(style.background.clone())
        .layout(PixelPerfectLayout::new((160, 144), (0, 0)))
        .child(ctx, |cb, _| {
            cb.graphic(Texture::new(textures.screen, [0.0, 0.0, 1.0, 1.0]))
        })
        .build(ctx);

    // create debug panel
    let debug_panel = ctx
        .create_control()
        .layout(VBoxLayout::default())
        .parent(split_view)
        .build(ctx);

    let tab_header = ctx
        .create_control()
        .parent(debug_panel)
        .layout(HBoxLayout::default())
        .min_size([16.0, 16.0])
        .build(ctx);

    let tab_page = ctx
        .create_control()
        .parent(debug_panel)
        .expand_y(true)
        .build(ctx);

    let tab_group = ButtonGroup::new(|_, _| ());

    let disas_page = ctx.create_control().parent(tab_page).build(ctx);
    disassembler_viewer::build(disas_page, ctx, &mut *event_table.borrow_mut(), &style);
    let _disas_tab = ctx
        .create_control()
        .parent(tab_header)
        .child(ctx, |cb, _| {
            cb.graphic(Text::new(
                "disassembly".to_string(),
                (0, 0),
                style.text_style.clone(),
            ))
            .layout(FitText)
        })
        .layout(MarginLayout::default())
        .behaviour(TabButton::new(
            tab_group.clone(),
            disas_page,
            true,
            style.tab_style.clone(),
        ))
        .build(ctx);

    let ppu_page = ctx.create_control().parent(tab_page).build(ctx);
    ppu_viewer::build(
        ppu_page,
        ctx,
        &mut *event_table.borrow_mut(),
        &style,
        textures,
    );
    let _ppu_tab = ctx
        .create_control()
        .parent(tab_header)
        .child(ctx, |cb, _| {
            cb.graphic(Text::new(
                "ppu".to_string(),
                (0, 0),
                style.text_style.clone(),
            ))
            .layout(FitText)
        })
        .layout(MarginLayout::default())
        .behaviour(TabButton::new(
            tab_group.clone(),
            ppu_page,
            false,
            style.tab_style.clone(),
        ))
        .build(ctx);

    let proxy = ctx.get::<EventLoopProxy<UserEvent>>();
    proxy.send_event(UserEvent::Debug(true)).unwrap();
}

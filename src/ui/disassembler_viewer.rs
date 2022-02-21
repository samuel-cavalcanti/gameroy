use std::{any::Any, ops::Range, sync::Arc};

use crui::{
    graphics::{Graphic, Text},
    layouts::{FitText, HBoxLayout, VBoxLayout},
    style::ButtonStyle,
    text::{Span, TextStyle},
    widgets::{
        Button, InteractiveText, ListBuilder, SetScrollPosition, TextField, TextFieldCallback,
        UpdateItems,
    },
    BuilderContext, Color, Context, ControlBuilder, Id, MouseEvent, MouseInfo,
};
use gameroy::{
    debugger::{break_flags, Debugger},
    disassembler::{Address, Directive},
    gameboy::GameBoy,
};
use parking_lot::Mutex;

// use std::{rc::Rc, cell::RefCell};
use crate::{
    event_table::{self, BreakpointsUpdated, EmulatorUpdated, EventTable, Handle, WatchsUpdated},
    fold_view::FoldView,
    style::Style,
};

struct Callback;
impl TextFieldCallback for Callback {
    fn on_submit(&mut self, _this: Id, ctx: &mut Context, text: &mut String) {
        let mut debugger = ctx.get::<Arc<Mutex<Debugger>>>().lock();
        let gb = ctx.get::<Arc<Mutex<GameBoy>>>().lock();
        let mut args: Vec<&str> = text.split_ascii_whitespace().collect();
        if args.len() == 0 {
            args.push("");
        }
        match debugger.execute_command(&*gb, &args) {
            Ok(_) => {}
            Err(x) => eprintln!("{}", x),
        }
        text.clear();
    }

    fn on_change(&mut self, _this: Id, _ctx: &mut Context, _text: &str) {}

    fn on_unfocus(&mut self, _this: Id, _ctx: &mut Context, _text: &mut String) {}
}

struct JumpToAddress {
    from_address: Address,
}

struct DissasemblerList {
    text_style: TextStyle,
    list: Id,
    reg: Id,
    pc: Option<Address>,
    directives: Vec<Directive>,
    items_are_dirty: bool,
    _emulator_updated_event: Handle<EmulatorUpdated>,
}
impl DissasemblerList {
    fn graphic(
        &mut self,
        direc: Directive,
        trace: std::cell::Ref<gameroy::disassembler::Trace>,
        pc: Option<Address>,
    ) -> (Graphic, Option<Range<usize>>) {
        let curr = direc.address;
        let mut text = format!(
            "{:04x} {:16} ",
            {
                let mut address = curr.address;
                if address < 0x4000 && curr.bank != 0 {
                    address += 0x4000;
                }
                address
            },
            trace
                .labels
                .get(&curr)
                .map(|x| x.name.as_str())
                .or_else(|| trace.ram_labels.get(&curr.address).map(|x| x.as_str()))
                .unwrap_or("")
        );
        let label = |pc, x| {
            if let Some(address) = trace.jumps.get(&pc) {
                let mut name = trace.labels.get(&address).unwrap().name.clone();
                name.insert_str(0, "<l>");
                name += "</l>";
                return name;
            }
            if let Some(name) = trace.ram_labels.get(&x) {
                let mut name = name.clone();
                name.insert_str(0, "<l>");
                name += "</l>";
                return name;
            }
            format!("<a>${:04x}</a>", x)
        };
        gameroy::disassembler::disassembly_opcode(
            direc.address.address,
            &direc.op[0..direc.len as usize],
            |x| label(curr, x),
            &mut text,
        )
        .unwrap();
        let style = self.text_style.clone();
        let label_range = if let Some(start) = text.find("<l>") {
            let end = text.find("</l>").unwrap() - 3;
            text.replace_range(start..start + 3, "");
            text.replace_range(end..end + 4, "");
            Some(start..end)
        } else {
            None
        };
        let address_range = if let Some(start) = text.find("<a>") {
            let end = text.find("</a>").unwrap() - 3;
            text.replace_range(start..start + 3, "");
            text.replace_range(end..end + 4, "");
            Some(start..end)
        } else {
            None
        };
        let op_len = text[22..].find(" ").unwrap();

        let mut text = Text::new(text, (-1, 0), style);

        let label = 0x2e8bb2ff.into();
        let op = 0xff1a1aff.into();
        let number = 0xd79314ff.into();
        let address = 0x6f7e67ff.into();

        text.add_span(0..4, Span::Color(address));
        text.add_span(5..21, Span::Color(label));
        label_range
            .as_ref()
            .map(|r| text.add_span(r.clone(), Span::Color(label)));
        text.add_span(22..22 + op_len, Span::Color(op));
        address_range.map(|r| text.add_span(r, Span::Color(number)));
        if Some(curr) == pc {
            text.add_span(
                0..text.len(),
                Span::Selection {
                    bg: Color::BLACK,
                    fg: None,
                },
            );
        }
        (text.into(), label_range)
    }
}
impl ListBuilder for DissasemblerList {
    fn on_event(&mut self, event: Box<dyn Any>, _this: Id, ctx: &mut Context) {
        if event.is::<EmulatorUpdated>() {
            let gb = ctx.get::<Arc<Mutex<GameBoy>>>().clone();
            let gb = gb.lock();

            fn decimal_mark(n: u64) -> String {
                let s = n.to_string();
                let mut result = String::with_capacity(s.len() + ((s.len() - 1) / 3));
                let mut i = s.len();
                for c in s.chars() {
                    result.push(c);
                    i -= 1;
                    if i > 0 && i % 3 == 0 {
                        result.push(' ');
                    }
                }
                result
            }

            let cpu = &gb.cpu;
            let reg_text = format!(
                "\
clock: {}
AF: {:02x} {:02x}
BC: {:02x} {:02x}
DE: {:02x} {:02x}
HL: {:02x} {:02x}
SP: {:04x}
PC: {:04x}",
                decimal_mark(gb.clock_count),
                cpu.a,
                cpu.f.0,
                cpu.b,
                cpu.c,
                cpu.d,
                cpu.e,
                cpu.h,
                cpu.l,
                cpu.sp,
                cpu.pc
            );

            let trace = gb.trace.borrow();

            self.items_are_dirty = true;
            self.directives.clear();
            self.directives.extend(trace.directives.iter().cloned());
            self.directives
                .extend(trace.ram_directives.iter().map(|&(address, op, len)| {
                    // TODO: I am violating my own rule about Address being only rom addresses.
                    // Maybe I should not have this rule, or have multiple address Types.
                    Directive {
                        address: Address {
                            bank: 0xFF,
                            address,
                        },
                        len: len as u16,
                        op,
                    }
                }));
            debug_assert!(self.directives.windows(2).all(|x| x[0] <= x[1]));

            let pc = cpu.pc;
            let bank = gb.cartridge.curr_bank();
            self.pc = Some(Address::from_pc(Some(bank), pc).unwrap_or(Address {
                address: pc,
                bank: 0xFF,
            }));
            let pc = self.pc.unwrap();

            let pos = self.directives.binary_search_by(|x| x.address.cmp(&pc));
            if let Ok(pos) = pos {
                let len = self.directives.len();
                let value = if len == 0 {
                    1.0
                } else {
                    pos as f32 / (len - 1) as f32
                };
                ctx.send_event_to(
                    self.list,
                    SetScrollPosition {
                        vertical: true,
                        value,
                    },
                );
            };

            if let Graphic::Text(text) = ctx.get_graphic_mut(self.reg) {
                text.set_string(&reg_text);
            }
        } else if let Some(JumpToAddress { from_address }) = event.downcast_ref::<JumpToAddress>() {
            let gb = ctx.get::<Arc<Mutex<GameBoy>>>().lock();
            let trace = gb.trace.borrow_mut();
            let jump_to = trace.jumps.get(&from_address).unwrap();
            dbg!(&jump_to);
            let pos = self
                .directives
                .binary_search_by(|x| x.address.cmp(&jump_to));
            drop(trace);
            drop(gb);
            if let Ok(pos) = pos {
                let len = self.directives.len();
                let value = if len == 0 {
                    1.0
                } else {
                    pos as f32 / (len - 1) as f32
                };
                ctx.send_event_to(
                    self.list,
                    SetScrollPosition {
                        vertical: true,
                        value,
                    },
                );
            };
        }
    }

    fn item_count(&mut self, _ctx: &mut dyn crui::BuilderContext) -> usize {
        self.directives.len()
    }

    fn create_item<'a>(
        &mut self,
        index: usize,
        _list_id: Id,
        cb: crui::ControlBuilder,
        ctx: &mut dyn crui::BuilderContext,
    ) -> crui::ControlBuilder {
        let inter = ctx.get::<Arc<Mutex<GameBoy>>>().lock();

        let trace = inter.trace.borrow();
        let directive = self.directives[index].clone();
        let (graphic, label_range) = self.graphic(directive.clone(), trace, self.pc);
        let cb = cb.graphic(graphic).layout(FitText);
        let mut span = 0;
        if let Some(label_range) = label_range {
            cb.behaviour(InteractiveText::new(vec![(
                label_range.clone(),
                Box::new(move |mouse: MouseInfo, this: Id, ctx: &mut Context| {
                    let text = match ctx.get_graphic_mut(this) {
                        Graphic::Text(x) => x,
                        _ => return,
                    };
                    match mouse.event {
                        MouseEvent::Enter => {
                            let label = 0x2e8bb2ff.into();
                            span = text.add_span(label_range.clone(), Span::Underline(Some(label)));
                        }
                        MouseEvent::Exit => {
                            text.remove_span(span);
                        }
                        _ if mouse.click() => ctx.send_event_to(
                            _list_id,
                            JumpToAddress {
                                from_address: dbg!(directive.address),
                            },
                        ),
                        _ => {}
                    }
                }),
            )]))
        } else {
            cb
        }
    }

    fn update_item(&mut self, _index: usize, _item_id: Id, _ctx: &mut dyn BuilderContext) -> bool {
        if self.items_are_dirty {
            false
        } else {
            true
        }
    }

    fn finished_layout(&mut self) {
        self.items_are_dirty = false;
    }
}

struct BreakpointList {
    text_style: TextStyle,
    button_style: std::rc::Rc<ButtonStyle>,
    _breakpoints_updated_event: Handle<BreakpointsUpdated>,
}
impl BreakpointList {
    fn get_text(ctx: &mut dyn BuilderContext, index: usize) -> String {
        let debugger = ctx.get::<Arc<Mutex<Debugger>>>().lock();
        let (address, flags) = debugger.breakpoints().iter().nth(index).unwrap();
        let flags = {
            let mut flags_str = String::new();
            let check = |c, flag| if flags & flag != 0 { c } else { '-' };

            use break_flags::*;
            flags_str.push(check('w', WRITE));
            flags_str.push(check('r', READ));
            flags_str.push(check('j', JUMP));
            flags_str.push(check('x', EXECUTE));

            flags_str
        };
        let text = format!("{} {:04x}", flags, address);
        text
    }
}
impl ListBuilder for BreakpointList {
    fn on_event(&mut self, event: Box<dyn Any>, this: Id, ctx: &mut Context) {
        if event.is::<event_table::BreakpointsUpdated>() {
            ctx.send_event_to(this, UpdateItems);
        }
    }

    fn item_count(&mut self, ctx: &mut dyn BuilderContext) -> usize {
        ctx.get::<Arc<Mutex<Debugger>>>().lock().breakpoints().len()
    }

    fn create_item<'a>(
        &mut self,
        index: usize,
        _list_id: Id,
        cb: ControlBuilder,
        ctx: &mut dyn BuilderContext,
    ) -> ControlBuilder {
        let text = Self::get_text(ctx, index);
        cb.layout(HBoxLayout::new(0.0, [0.0; 4], 1))
            .child(ctx, |cb, _| {
                cb.graphic(Text::new(text, (-1, 0), self.text_style.clone()))
                    .layout(FitText)
                    .expand_x(true)
            })
            .child(ctx, |cb, _| {
                cb.behaviour(Button::new(
                    self.button_style.clone(),
                    true,
                    move |_, ctx| {
                        let mut debugger = ctx.get::<Arc<Mutex<Debugger>>>().lock();
                        let &address = debugger.breakpoints().keys().nth(index).unwrap();
                        debugger.remove_break(address);
                    },
                ))
                .min_size([15.0, 15.0])
                .fill_y(crui::RectFill::ShrinkCenter)
            })
    }

    fn update_item(&mut self, index: usize, item_id: Id, ctx: &mut dyn BuilderContext) -> bool {
        let text = Self::get_text(ctx, index);
        let text_id = ctx.get_active_children(item_id)[0];
        if let Graphic::Text(x) = ctx.get_graphic_mut(text_id) {
            x.set_string(&text);
        }
        true
    }
}

struct WatchsList {
    text_style: TextStyle,
    button_style: std::rc::Rc<ButtonStyle>,
    _watchs_updated_event: Handle<WatchsUpdated>,
    _emulator_updated_event: Handle<EmulatorUpdated>,
}
impl WatchsList {
    fn watch_text(ctx: &mut dyn BuilderContext, index: usize) -> (u16, String) {
        let &address = ctx
            .get::<Arc<Mutex<Debugger>>>()
            .lock()
            .watchs()
            .iter()
            .nth(index)
            .unwrap();
        let value = ctx.get::<Arc<Mutex<GameBoy>>>().lock().read(address);
        let text = format!("{:04x} = {:02x}", address, value);
        (address, text)
    }
}
impl ListBuilder for WatchsList {
    fn on_event(&mut self, event: Box<dyn Any>, this: Id, ctx: &mut Context) {
        if event.is::<event_table::WatchsUpdated>() || event.is::<event_table::EmulatorUpdated>() {
            ctx.send_event_to(this, UpdateItems);
        }
    }

    fn item_count(&mut self, ctx: &mut dyn BuilderContext) -> usize {
        ctx.get::<Arc<Mutex<Debugger>>>().lock().watchs().len()
    }

    fn create_item<'a>(
        &mut self,
        index: usize,
        _list_id: Id,
        cb: ControlBuilder,
        ctx: &mut dyn BuilderContext,
    ) -> ControlBuilder {
        let (address, text) = Self::watch_text(ctx, index);
        cb.layout(HBoxLayout::new(0.0, [0.0; 4], 1))
            .child(ctx, |cb, _| {
                cb.graphic(Text::new(text, (-1, 0), self.text_style.clone()))
                    .layout(FitText)
                    .expand_x(true)
            })
            .child(ctx, |cb, _| {
                cb.behaviour(Button::new(
                    self.button_style.clone(),
                    true,
                    move |_, ctx| {
                        let mut debugger = ctx.get::<Arc<Mutex<Debugger>>>().lock();
                        debugger.remove_watch(address);
                    },
                ))
                .min_size([15.0, 15.0])
                .fill_y(crui::RectFill::ShrinkCenter)
            })
    }

    fn update_item(&mut self, index: usize, item_id: Id, ctx: &mut dyn BuilderContext) -> bool {
        let (_, text) = Self::watch_text(ctx, index);
        let text_id = ctx.get_active_children(item_id)[0];
        if let Graphic::Text(x) = ctx.get_graphic_mut(text_id) {
            x.set_string(&text);
        }
        true
    }
}

pub fn build(
    parent: Id,
    ctx: &mut dyn BuilderContext,
    event_table: &mut EventTable,
    style: &Style,
) {
    let diss_view_id = ctx.reserve();
    let list_id = ctx.reserve();
    let reg_id = ctx.reserve();

    let vbox = ctx
        .create_control_reserved(diss_view_id)
        .graphic(style.background.clone())
        .parent(parent)
        .expand_y(true)
        .expand_x(true)
        .min_width(100.0)
        .layout(VBoxLayout::new(2.0, [2.0; 4], -1))
        .build(ctx);

    let h_box = ctx
        .create_control()
        .parent(vbox)
        .layout(HBoxLayout::default())
        .expand_y(true)
        .build(ctx);

    let ignore_min_width = ctx.create_control().parent(h_box).expand_x(true).build(ctx);

    list(
        ctx.create_control_reserved(list_id),
        ctx,
        style,
        DissasemblerList {
            text_style: style.text_style.clone(),
            list: list_id,
            reg: reg_id,
            pc: None,
            directives: Vec::new(),
            items_are_dirty: true,
            _emulator_updated_event: event_table.register(list_id),
        },
    )
    .parent(ignore_min_width)
    .build(ctx);

    let caret = ctx.reserve();
    let label = ctx.reserve();

    let right_panel = ctx
        .create_control()
        .parent(h_box)
        .layout(VBoxLayout::new(1.0, [2.0; 4], -1))
        .graphic(style.split_background.clone())
        .build(ctx);

    let _reg_view = ctx
        .create_control_reserved(reg_id)
        .parent(right_panel)
        .graphic(Text::new(
            format!(
                "\
AF: {:02x} {:02x}
BC: {:02x} {:02x}
DE: {:02x} {:02x}
HL: {:02x} {:02x}
SP: {:04x}
PC: {:04x}",
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ),
            (-1, 0),
            style.text_style.clone(),
        ))
        .layout(FitText)
        .build(ctx);

    let breaks = ctx
        .create_control()
        .parent(right_panel)
        .behaviour(FoldView { fold: false })
        .layout(VBoxLayout::new(1.0, [0.0; 4], -1))
        .build(ctx);

    let _break_header = ctx
        .create_control()
        .parent(breaks)
        .graphic(Text::new(
            "breakpoints".to_string(),
            (-1, 0),
            style.text_style.clone(),
        ))
        .layout(FitText)
        .build(ctx);

    let break_list = ctx.reserve();
    list(
        ctx.create_control_reserved(break_list)
            .parent(breaks)
            .min_size([50.0, 100.0]),
        ctx,
        style,
        BreakpointList {
            text_style: style.text_style.clone(),
            button_style: style.delete_button.clone(),
            _breakpoints_updated_event: event_table.register(break_list),
        },
    )
    .build(ctx);

    let watchs = ctx
        .create_control()
        .parent(right_panel)
        .behaviour(FoldView { fold: false })
        .layout(VBoxLayout::new(1.0, [0.0; 4], -1))
        .build(ctx);

    let _watchs_header = ctx
        .create_control()
        .parent(watchs)
        .graphic(Text::new(
            "watchs".to_string(),
            (-1, 0),
            style.text_style.clone(),
        ))
        .layout(FitText)
        .build(ctx);

    let watchs_list = ctx.reserve();
    list(
        ctx.create_control_reserved(watchs_list)
            .parent(watchs)
            .min_size([50.0, 100.0]),
        ctx,
        style,
        WatchsList {
            text_style: style.text_style.clone(),
            button_style: style.delete_button.clone(),
            _watchs_updated_event: event_table.register(watchs_list),
            _emulator_updated_event: event_table.register(watchs_list),
        },
    )
    .build(ctx);

    let text_field = ctx
        .create_control()
        .parent(vbox)
        .behaviour(TextField::new(
            caret,
            label,
            style.text_field.clone(),
            Callback,
        ))
        .min_size([20.0; 2])
        .focus(true)
        .build(ctx);

    ctx.create_control_reserved(caret)
        .parent(text_field)
        .graphic(style.background.clone().with_color([0, 0, 0, 255].into()))
        .anchors([0.0; 4])
        .build(ctx);

    ctx.create_control_reserved(label)
        .parent(text_field)
        .graphic(Text::new(String::new(), (-1, -1), style.text_style.clone()))
        .build(ctx);
}

fn list(
    cb: ControlBuilder,
    ctx: &mut (impl BuilderContext + ?Sized),
    style: &Style,
    list_builder: impl ListBuilder + 'static,
) -> ControlBuilder {
    use crui::widgets::{List, ScrollBar, ViewLayout};
    let scroll_view = cb.id();
    let view = ctx
        .create_control()
        .parent(scroll_view)
        .layout(ViewLayout::new(false, true))
        .build(ctx);
    let h_scroll_bar_handle = ctx.reserve();
    let h_scroll_bar = ctx
        .create_control()
        .min_size([10.0, 10.0])
        .parent(scroll_view)
        .behaviour(ScrollBar::new(
            h_scroll_bar_handle,
            scroll_view,
            false,
            style.scrollbar.clone(),
        ))
        .build(ctx);
    let h_scroll_bar_handle = ctx
        .create_control_reserved(h_scroll_bar_handle)
        .min_size([10.0, 10.0])
        .parent(h_scroll_bar)
        .build(ctx);
    let v_scroll_bar_handle = ctx.reserve();
    let v_scroll_bar = ctx
        .create_control()
        .min_size([10.0, 10.0])
        // .graphic(style.scroll_background.clone())
        .parent(scroll_view)
        .behaviour(ScrollBar::new(
            v_scroll_bar_handle,
            scroll_view,
            true,
            style.scrollbar.clone(),
        ))
        .build(ctx);
    let v_scroll_bar_handle = ctx
        .create_control_reserved(v_scroll_bar_handle)
        .min_size([10.0, 10.0])
        .parent(v_scroll_bar)
        .build(ctx);

    cb.behaviour_and_layout(List::new(
        10.0,
        0.0,
        [10.0, 0.0, 0.0, 0.0],
        view,
        v_scroll_bar,
        v_scroll_bar_handle,
        h_scroll_bar,
        h_scroll_bar_handle,
        list_builder,
    ))
}

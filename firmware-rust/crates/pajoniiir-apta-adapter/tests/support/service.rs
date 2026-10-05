// SPDX-License-Identifier: Apache-2.0
use pajoniiir_apta_adapter::service::{Authority, Command, Port, Retained, Status};
use pajoniiir_deck::DeckProductState;
use pajoniiir_media_session::MediaSession;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};
pub struct State<'a> {
    pub commands: RefCell<VecDeque<Command<'a>>>,
    pub outputs: RefCell<VecDeque<Retained<'a>>>,
    pub media: Cell<MediaSession>,
    pub decks: RefCell<DeckProductState>,
    pub authority: Cell<Authority>,
    pub status: Cell<Option<Status>>,
    pub accept: Cell<bool>,
    pub refuse: Cell<bool>,
    pub published: Cell<usize>,
}
impl<'a> State<'a> {
    pub fn new(media: MediaSession) -> Self {
        Self {
            commands: RefCell::new(VecDeque::with_capacity(2)),
            outputs: RefCell::new(VecDeque::with_capacity(2)),
            media: Cell::new(media),
            decks: RefCell::new(DeckProductState::new()),
            authority: Cell::new(Authority::EMPTY),
            status: Cell::new(None),
            accept: Cell::new(true),
            refuse: Cell::new(false),
            published: Cell::new(0),
        }
    }
    #[allow(clippy::result_large_err)]
    pub fn send(&self, command: Command<'a>) -> Result<(), Command<'a>> {
        let mut queue = self.commands.borrow_mut();
        if queue.len() == 2 {
            return Err(command);
        }
        queue.push_back(command);
        Ok(())
    }
}
pub struct HostPort<'a>(pub Rc<State<'a>>);
impl<'a> Port<'a> for HostPort<'a> {
    async fn receive(&mut self) -> Command<'a> {
        core::future::poll_fn(|cx| {
            if let Some(command) = self.0.commands.borrow_mut().pop_front() {
                core::task::Poll::Ready(command)
            } else {
                cx.waker().wake_by_ref();
                core::task::Poll::Pending
            }
        })
        .await
    }
    fn media(&self) -> MediaSession {
        self.0.media.get()
    }
    fn authority(&mut self, authority: Authority) {
        self.0.authority.set(authority);
    }
    fn publish(
        &mut self,
        mut output: Retained<'a>,
    ) -> Result<(), pajoniiir_apta_adapter::owner::Output<'a>> {
        if self.0.refuse.get() || self.0.outputs.borrow().len() == 2 {
            return Err(output.reject().ok().unwrap());
        }
        if self.0.accept.get()
            && output
                .accept(
                    self.0.authority.get(),
                    &self.0.media.get(),
                    &self.0.decks.borrow(),
                )
                .is_err()
        {
            return Err(output.reject().ok().unwrap());
        }
        self.0.outputs.borrow_mut().push_back(output);
        self.0.published.set(self.0.published.get() + 1);
        Ok(())
    }
    fn report(&mut self, status: Status) {
        self.0.status.set(Some(status));
    }
}

#![allow(async_fn_in_trait)]

pub type Sender<T> = crossbeam_channel::Sender<T>;

pub type Receiver<T> = crossbeam_channel::Receiver<T>;

pub trait Connections {
    type Senders;
    type Receivers;

    fn receiver<T>(&mut self) -> &mut Receiver<T>
    where
        Self::Receivers: AsMut<Receiver<T>>;

    fn receive<T, F>(&mut self, mut handler: F) -> bool
    where
        Self::Receivers: AsMut<Receiver<T>>,
        F: FnMut(T, &Self::Senders),
    {
        let receiver = self.receiver();
        let message = receiver.recv().ok();
        let Some(message) = message else {
            return false;
        };
        handler(message, self.senders());
        true
    }

    async fn try_receive<T, F>(&mut self, mut handler: F) -> bool
    where
        Self::Receivers: AsMut<Receiver<T>>,
        F: AsyncFnMut(T, &Self::Senders),
    {
        let receiver = self.receiver();
        let message = receiver.try_recv().ok();
        let Some(message) = message else {
            return false;
        };
        handler(message, self.senders()).await;
        true
    }

    fn senders(&self) -> &Self::Senders;

    fn sender<T>(&mut self) -> &Sender<T>
    where
        Self::Senders: AsRef<Sender<T>>;

    fn send<T>(&mut self, message: T) -> Result<(), crossbeam_channel::SendError<T>>
    where
        Self::Senders: AsRef<Sender<T>>,
    {
        let sender = self.sender();
        sender.send(message)
    }

    fn try_send<T>(&mut self, message: T) -> Result<(), crossbeam_channel::TrySendError<T>>
    where
        Self::Senders: AsRef<Sender<T>>,
    {
        let sender = self.sender();
        sender.try_send(message)
    }
}

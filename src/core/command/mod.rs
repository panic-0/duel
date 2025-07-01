use super::{buff::Buff, event::Event, player::Player, world::World, BuffId, PlayerId};

pub trait Command: std::fmt::Debug {
    fn apply(self: Box<Self>, world: &mut World);
}

#[derive(Debug, Default)]
pub struct Commands {
    pub commands: Vec<Box<dyn Command>>,
}

impl Commands {
    pub fn push<T: Command + 'static>(&mut self, command: T) {
        self.commands.push(Box::new(command));
    }
}

#[derive(Debug)]
pub struct AddPlayer {
    pub player: Player,
}

impl Command for AddPlayer {
    fn apply(self: Box<Self>, world: &mut World) {
        world.add_player(self.player);
    }
}

#[derive(Debug)]
pub struct RemovePlayer {
    pub id: PlayerId,
}

impl Command for RemovePlayer {
    fn apply(self: Box<Self>, world: &mut World) {
        world.remove_player(self.id);
    }
}

#[derive(Debug)]
pub struct AddBuff {
    pub buff: Box<dyn Buff>,
}

impl Command for AddBuff {
    fn apply(self: Box<Self>, world: &mut World) {
        world.add_buff(self.buff);
    }
}

#[derive(Debug)]
pub struct RemoveBuff {
    pub id: BuffId,
}

impl Command for RemoveBuff {
    fn apply(self: Box<Self>, world: &mut World) {
        world.remove_buff(self.id);
    }
}

#[derive(Debug)]
pub struct ApplyEvent {
    pub event: Event,
}

impl Command for ApplyEvent {
    fn apply(self: Box<Self>, world: &mut World) {
        world.apply_event(self.event);
    }
}

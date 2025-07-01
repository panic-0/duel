use super::{
    buff::{get_event_priorities, Buff, BuffType},
    command::Commands,
    event::{Event, EventType},
    player::Player,
    BuffId, PlayerId,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use strum::IntoEnumIterator;

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventRegistration {
    priority: u32,
    buff_id: BuffId,
    buff_type: BuffType,
}

impl PartialOrd for EventRegistration {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::cmp::Ord for EventRegistration {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.buff_id.cmp(&other.buff_id))
    }
}

#[derive(Debug)]
pub struct World {
    players: BTreeMap<PlayerId, Player>,
    player_id_counter: PlayerId,
    buffs: BTreeMap<BuffId, Box<dyn Buff>>,
    buff_id_counter: BuffId,
    event_registry: HashMap<EventType, BTreeSet<EventRegistration>>,
    buff_to_events: HashMap<BuffId, Vec<EventType>>,
    end: bool,
}

impl World {
    pub fn new() -> Self {
        World {
            players: BTreeMap::new(),
            player_id_counter: 0,
            buffs: BTreeMap::new(),
            buff_id_counter: 0,
            event_registry: HashMap::new(),
            buff_to_events: HashMap::new(),
            end: false,
        }
    }

    pub fn add_player(&mut self, player: Player) -> PlayerId {
        let id = self.player_id_counter;
        self.player_id_counter += 1;
        self.players.insert(id, player);
        id
    }

    pub fn remove_player(&mut self, id: PlayerId) -> Option<Player> {
        self.players.remove(&id)
    }

    pub fn get_players(&self) -> &BTreeMap<PlayerId, Player> {
        &self.players
    }

    pub fn get_player(&self, id: PlayerId) -> Option<&Player> {
        self.players.get(&id)
    }

    pub fn get_player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.get_mut(&id)
    }

    pub fn add_buff(&mut self, buff: Box<dyn Buff>) -> BuffId {
        let id = self.buff_id_counter;
        self.buff_id_counter += 1;

        let buff_type = buff.buff_type();
        let mut registered_events = Vec::new();
        for event_type in EventType::iter() {
            let priorities = get_event_priorities(event_type);
            let index = priorities.iter().position(|&bt| bt == buff_type);
            if let Some(index) = index {
                let priority = index as u32;
                let registrations = self.event_registry.entry(event_type).or_default();
                registrations.insert(EventRegistration {
                    priority,
                    buff_id: id,
                    buff_type,
                });
                registered_events.push(event_type);
            }
        }

        if !registered_events.is_empty() {
            self.buff_to_events.insert(id, registered_events);
        }

        self.buffs.insert(id, buff);
        id
    }

    pub fn remove_buff(&mut self, id: BuffId) -> Option<Box<dyn Buff>> {
        if let Some(buff) = self.buffs.remove(&id) {
            if let Some(event_types) = self.buff_to_events.remove(&id) {
                for event_type in event_types {
                    if let Some(registrations) = self.event_registry.get_mut(&event_type) {
                        registrations.retain(|reg| reg.buff_id != id);
                    }
                }
            }
            Some(buff)
        } else {
            None
        }
    }

    pub fn get_buffs(&self) -> &BTreeMap<BuffId, Box<dyn Buff>> {
        &self.buffs
    }

    pub fn get_buff(&self, id: BuffId) -> Option<&dyn Buff> {
        self.buffs.get(&id).map(|b| b.as_ref())
    }

    pub fn get_buff_mut(&mut self, id: BuffId) -> Option<&mut Box<dyn Buff>> {
        self.buffs.get_mut(&id)
    }

    pub fn apply_event(&mut self, event: &mut Event) {
        if self.is_end() {
            return;
        }
        let mut commands = Commands::default();

        if let Some(registrations) = self.event_registry.get(&event.event_type()) {
            for registration in registrations {
                if let Some(buff) = self.buffs.get(&registration.buff_id) {
                    buff.on_event(event, self, &mut commands, registration.buff_id);
                }
            }
        }

        commands
            .commands
            .into_iter()
            .for_each(|command| command.apply(self));

        if self.players.len() <= 1 {
            self.end = true;
        }
    }

    pub fn is_end(&self) -> bool {
        self.end
    }
}

impl World {
    pub fn get_next_player_not_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.players.range(id + 1..).next().map(|(id, _)| *id)
    }

    pub fn get_next_player_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.players
            .range(id + 1..)
            .next()
            .or_else(|| self.players.range(..id).next())
            .map(|(id, _)| *id)
    }

    pub fn run(mut self) {
        self.add_buff(Box::new(super::buff::StateMachine));
        self.apply_event(&mut Event::DuelStart);
    }
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

impl World {
    pub fn get_registry_stats(&self) -> (usize, usize, usize) {
        let total_events = self.event_registry.len();
        let total_registrations: usize = self.event_registry.values().map(|set| set.len()).sum();
        let total_buffs = self.buffs.len();
        (total_events, total_registrations, total_buffs)
    }

    pub fn get_event_registration_count(&self, event_type: EventType) -> usize {
        self.event_registry
            .get(&event_type)
            .map(|set| set.len())
            .unwrap_or(0)
    }

    pub fn validate_registry_consistency(&self) -> bool {
        for (buff_id, event_types) in &self.buff_to_events {
            if !self.buffs.contains_key(buff_id) {
                return false;
            }

            for &event_type in event_types {
                if let Some(registrations) = self.event_registry.get(&event_type) {
                    if !registrations.iter().any(|reg| reg.buff_id == *buff_id) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }

        for registrations in self.event_registry.values() {
            for registration in registrations {
                if !self.buffs.contains_key(&registration.buff_id) {
                    return false;
                }

                if !self.buff_to_events.contains_key(&registration.buff_id) {
                    return false;
                }
            }
        }

        true
    }
}

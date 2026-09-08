pub mod core;

use core::{
    business::flow::DuelRunner,
    business::{damage_reduction, revival},
    log::{LogEntry, Logger},
    player::Player,
    state::GameState,
    world::{World, MAX_ROUNDS},
    Abilities, Attack, PlayerId,
};

pub struct GameBuilder {
    world: World,
}

impl GameBuilder {
    pub fn new() -> Self {
        Self {
            world: World::new(),
        }
    }

    pub fn add_player(mut self, name: &str, hp: u64, attack: u64) -> (Self, PlayerId) {
        let player = Player::new(name.to_string(), hp, attack);
        let player_id = self.world.add_player(player);
        (self, player_id)
    }

    pub fn build(self) -> World {
        self.world
    }
}

impl Default for GameBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn print_log(world: &GameState, entry: &LogEntry) {
    use colored::Colorize;
    let name = |id: PlayerId| {
        world
            .get_player(id)
            .map(|p| p.name().to_string())
            .unwrap_or_default()
    };
    match entry {
        LogEntry::Attack {
            source_id,
            target_id,
        } => println!(
            "{} 对 {} 进行了普通攻击",
            name(*source_id).blue(),
            name(*target_id).blue()
        ),
        LogEntry::DamageReduced {
            target_id,
            original,
            reduced,
        } => println!(
            "{} 的减伤效果触发！伤害从 {} 降低到 {}",
            name(*target_id).blue(),
            original.to_string().bright_red(),
            reduced.to_string().yellow()
        ),
        LogEntry::Damage {
            target_id,
            amount,
            hp,
            max_hp,
        } => println!(
            "{} 受到了 {} 点伤害 ({}/{} HP)",
            name(*target_id).blue(),
            amount.to_string().bright_red(),
            hp,
            max_hp
        ),
        LogEntry::Heal {
            target_id,
            amount,
            hp,
            max_hp,
        } => println!(
            "{} 回复了 {} 点生命 ({}/{} HP)",
            name(*target_id).blue(),
            amount.to_string().bright_green(),
            hp,
            max_hp
        ),
        LogEntry::Revival { player_id } => println!("{} 复活了！", name(*player_id).blue()),
        LogEntry::Death { player_id } => println!("{} 已经死亡", name(*player_id).bright_red()),
        LogEntry::Draw => println!("已达到最大回合数（{}），平局", MAX_ROUNDS),
    }
}

fn main() {
    let builder = GameBuilder::new();
    let (builder, player1_id) = builder.add_player("Player1", 15, 10);
    let (builder, player2_id) = builder.add_player("Player2", 28, 8);
    let mut world = builder.build();

    // 技能集合是数据实例，owner 随玩家销毁。
    world.add_data(Some(player1_id), Abilities::new(vec![Box::new(Attack)]));
    world.add_data(Some(player2_id), Abilities::new(vec![Box::new(Attack)]));

    // 救回：System 注册一次，实例按玩家添加。
    revival::register_revival_system(&mut world);
    revival::add_revival(&mut world, player1_id);

    // 减伤：owner 与 target 都指向 Player2；跨角色示例见回归测试。
    damage_reduction::register_damage_reduction_rule(&mut world);
    damage_reduction::add_damage_reduction(&mut world, player2_id, player2_id, 0.2);

    core::install_default_rules(&mut world);

    world.set_logger(Logger::new(Box::new(print_log)));
    world.run().expect("对局应正常执行");
}

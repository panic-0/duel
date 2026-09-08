//! 命令行示例的对局装配与日志展示。

mod builder;
mod log;

use builder::GameBuilder;
use duel::core::{
    business::{damage_reduction, revival},
    log::Logger,
    Abilities, Attack, DuelRunner,
};
use log::print_log;

pub(crate) fn run() {
    let builder = GameBuilder::new();
    let (builder, player1_id) = builder.add_player("Player1", 15, 10);
    let (builder, player2_id) = builder.add_player("Player2", 28, 8);
    let mut world = builder.build();

    // 技能集合是数据实例，owner 随玩家销毁。
    world.add_data(
        Some(player1_id),
        Abilities::new(player1_id, vec![Box::new(Attack)]),
    );
    world.add_data(
        Some(player2_id),
        Abilities::new(player2_id, vec![Box::new(Attack)]),
    );

    // 救回：System 注册一次，实例按玩家添加。
    revival::register_revival_system(&mut world);
    revival::add_revival(&mut world, player1_id);

    // 减伤：owner 与 target 都指向 Player2；跨角色示例见回归测试。
    damage_reduction::register_damage_reduction_rule(&mut world);
    damage_reduction::add_damage_reduction(&mut world, player2_id, player2_id, 0.2);

    duel::core::install_default_rules(&mut world);

    world.set_logger(Logger::new(Box::new(print_log)));
    world.run().expect("对局应正常执行");
}

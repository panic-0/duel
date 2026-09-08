//! 命令行示例的对局装配与日志展示。

mod builder;
mod log;

use builder::GameBuilder;
use duel::core::{
    attach_taunt,
    business::{damage_reduction, revival},
    log::Logger,
    Abilities, Attack, Combo, DuelRunner,
};
use log::print_log;

pub(crate) fn run() {
    let builder = GameBuilder::new();
    let (builder, guardian_id) = builder.add_player("Guardian", 32, 3);
    let (builder, berserker_id) = builder.add_player("Berserker", 24, 5);
    let (builder, duelist_id) = builder.add_player("Duelist", 20, 4);
    let mut world = builder.build();

    // 守护者有普通攻击；狂战士一次行动连续攻击两次；决斗者使用普通攻击。
    world.attach_component(
        Some(guardian_id),
        Abilities::new(guardian_id, vec![Box::new(Attack)]),
    );
    world.attach_component(
        Some(berserker_id),
        Abilities::new(berserker_id, vec![Box::new(Combo::new(2))]),
    );
    world.attach_component(
        Some(duelist_id),
        Abilities::new(duelist_id, vec![Box::new(Attack)]),
    );

    // 守护者吸引攻击；其 owner 死亡时，嘲讽数据会随生命周期一起销毁。
    attach_taunt(&mut world, guardian_id);

    // 救回：System 注册一次，实例按玩家添加。守护者死亡后会恢复一半生命。
    revival::register_revival_system(&mut world);
    revival::attach_revival(&mut world, guardian_id);

    // 减伤：守护者受到的伤害降低 25%。
    damage_reduction::register_damage_reduction_rule(&mut world);
    damage_reduction::attach_damage_reduction(&mut world, guardian_id, guardian_id, 0.25);

    duel::core::install_default_rules(&mut world);

    world.set_logger(Logger::new(Box::new(print_log)));
    world.run().expect("对局应正常执行");
}

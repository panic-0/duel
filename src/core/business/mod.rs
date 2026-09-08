//! 业务层：Damage、死亡、胜负、技能与流程等玩法规则。
//! 依赖方向：业务认识业务数据与上下文；基础执行器与 System 协议不认识任何业务类型。

pub mod attack;
pub mod damage;
pub mod damage_reduction;
pub mod death;
pub mod flow;
pub mod heal;
pub mod revival;
pub mod skills;
pub mod victory;

use super::world::World;

/// 显式装配默认玩法规则：零血死亡判断与最后一人生还的胜负检查。
/// 自定义装配可以只注册其中一部分；装配本身不改变执行协议。
pub fn install_default_rules(world: &mut World) {
    world.add_system(death::DeathSystem);
    world.add_system(victory::VictorySystem);
}

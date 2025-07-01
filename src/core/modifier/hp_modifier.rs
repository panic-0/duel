use super::*;

#[derive(Debug)]
pub struct HpModifier {
    target_id: PlayerId,
    modifier: i64,
}

impl HpModifier {
    pub fn new(target_id: PlayerId, modifier: i64) -> Self {
        HpModifier {
            target_id,
            modifier,
        }
    }

    pub fn damage(target_id: PlayerId, amount: u64) -> Self {
        Self::new(target_id, -(amount as i64))
    }

    pub fn heal(target_id: PlayerId, amount: u64) -> Self {
        Self::new(target_id, amount as i64)
    }
}

impl Command for HpModifier {
    fn apply(self: Box<Self>, world: &mut World) {
        let Some(target) = world.get_player_mut(self.target_id) else {
            return;
        };

        let old_hp = target.hp();
        let new_hp = target.modify_hp(self.modifier);

        // 显示伤害/治疗信息
        use std::cmp::Ordering;
        match self.modifier.cmp(&0) {
            Ordering::Less => {
                println!(
                    "{} 受到了 {} 点伤害 ({}/{} HP)",
                    target.name().blue(),
                    (-self.modifier).to_string().bright_red(),
                    new_hp,
                    target.max_hp()
                );
            }
            Ordering::Greater => {
                println!(
                    "{} 回复了 {} 点生命 ({}/{} HP)",
                    target.name().blue(),
                    self.modifier.to_string().bright_green(),
                    new_hp,
                    target.max_hp()
                );
            }
            Ordering::Equal => {}
        }

        // 检查死亡 - 只触发事件，不直接操作 world
        if old_hp > 0 && new_hp == 0 {
            // 通过命令系统处理死亡事件，而不是直接调用
            world.apply_event(Event::BeforePlayerDeath(self.target_id));

            // 再次检查玩家是否仍然死亡（可能被复活技能救活）
            if let Some(target) = world.get_player(self.target_id) {
                if target.hp() == 0 {
                    println!("{} 已经死亡", target.name().bright_red());
                    world.apply_event(Event::AfterPlayerDeath(self.target_id));
                    // 死亡后移除玩家应该通过专门的命令处理
                    world.remove_player(self.target_id);
                }
            }
        }
    }
}

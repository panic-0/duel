use super::super::log::LogEntry;
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
        let (old_hp, new_hp, max_hp) = {
            let Some(target) = world.get_player_mut(self.target_id) else {
                return;
            };
            let old_hp = target.hp();
            let new_hp = target.modify_hp(self.modifier);
            (old_hp, new_hp, target.max_hp())
        };

        // 显示伤害/治疗信息
        use std::cmp::Ordering;
        match self.modifier.cmp(&0) {
            Ordering::Less => world.log(LogEntry::Damage {
                target_id: self.target_id,
                amount: (-self.modifier) as u64,
                hp: new_hp,
                max_hp,
            }),
            Ordering::Greater => world.log(LogEntry::Heal {
                target_id: self.target_id,
                amount: self.modifier as u64,
                hp: new_hp,
                max_hp,
            }),
            Ordering::Equal => {}
        }

        // 血量跨越 0 只报告事实；死亡判定、终局通知与移除由引擎的死亡结算处理
        if old_hp > 0 && new_hp == 0 {
            world.queue_event(Event::BeforePlayerDeath(self.target_id));
        }
    }
}

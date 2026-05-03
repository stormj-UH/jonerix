//! CARP election state translated from OpenBSD's `carp_proto_input_c`,
//! `carp_master_down`, and `carp_setrun` behavior.

use std::time::Duration;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CarpState {
    Init,
    Backup,
    Master,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LocalNode {
    pub vhid: u8,
    pub advbase: u8,
    pub advskew: u8,
    pub demote: u8,
    pub preempt: bool,
    pub state: CarpState,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RemoteAdvertisement {
    pub advbase: u8,
    pub advskew: u8,
    pub demote: u8,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Ignore,
    StayMaster,
    BecomeBackup,
    BecomeMaster,
    ResetMasterDownTimer,
}

impl LocalNode {
    pub fn new(vhid: u8, advbase: u8, advskew: u8, demote: u8, preempt: bool) -> Self {
        Self {
            vhid,
            advbase,
            advskew,
            demote,
            preempt,
            state: CarpState::Init,
        }
    }

    pub fn local_interval(&self) -> Duration {
        advertisement_interval(self.advbase, self.advskew)
    }

    pub fn master_down_timeout(&self) -> Duration {
        self.local_interval() * 3
    }

    pub fn start(&mut self) {
        if self.state == CarpState::Init {
            self.state = CarpState::Backup;
        }
    }

    pub fn master_down(&mut self) -> Decision {
        if self.state == CarpState::Backup {
            self.state = CarpState::Master;
            Decision::BecomeMaster
        } else {
            Decision::Ignore
        }
    }

    pub fn observe_advertisement(&mut self, remote: RemoteAdvertisement) -> Decision {
        let local_iv = self.local_interval();
        let remote_iv = advertisement_interval(remote.advbase, remote.advskew);

        match self.state {
            CarpState::Init => Decision::Ignore,
            CarpState::Master => {
                if ((local_iv >= remote_iv) && remote.demote <= self.demote)
                    || remote.demote < self.demote
                {
                    self.state = CarpState::Backup;
                    Decision::BecomeBackup
                } else {
                    Decision::StayMaster
                }
            }
            CarpState::Backup => {
                if self.preempt && local_iv < remote_iv && remote.demote >= self.demote {
                    self.state = CarpState::Master;
                    return Decision::BecomeMaster;
                }
                if remote.demote > self.demote {
                    self.state = CarpState::Master;
                    return Decision::BecomeMaster;
                }
                if self.advbase != 0 && Duration::from_secs(self.advbase as u64 * 3) < remote_iv {
                    self.state = CarpState::Master;
                    return Decision::BecomeMaster;
                }
                Decision::ResetMasterDownTimer
            }
        }
    }
}

pub fn advertisement_interval(advbase: u8, advskew: u8) -> Duration {
    let micros = advbase as u64 * 1_000_000 + advskew as u64 * 1_000_000 / 256;
    Duration::from_micros(if micros == 0 { 1_000_000 / 256 } else { micros })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_uses_openbsd_skew_formula() {
        assert_eq!(
            advertisement_interval(1, 128),
            Duration::from_micros(1_500_000)
        );
        assert_eq!(advertisement_interval(0, 0), Duration::from_micros(3_906));
    }

    #[test]
    fn master_backs_down_to_equal_or_better_remote() {
        let mut local = LocalNode::new(1, 1, 100, 0, false);
        local.state = CarpState::Master;
        let decision = local.observe_advertisement(RemoteAdvertisement {
            advbase: 1,
            advskew: 100,
            demote: 0,
        });
        assert_eq!(decision, Decision::BecomeBackup);
        assert_eq!(local.state, CarpState::Backup);
    }

    #[test]
    fn backup_preempts_slower_master_when_enabled() {
        let mut local = LocalNode::new(1, 1, 0, 0, true);
        local.state = CarpState::Backup;
        let decision = local.observe_advertisement(RemoteAdvertisement {
            advbase: 1,
            advskew: 200,
            demote: 0,
        });
        assert_eq!(decision, Decision::BecomeMaster);
        assert_eq!(local.state, CarpState::Master);
    }

    #[test]
    fn backup_takes_over_higher_remote_demote_even_without_preempt() {
        let mut local = LocalNode::new(1, 1, 200, 1, false);
        local.state = CarpState::Backup;
        let decision = local.observe_advertisement(RemoteAdvertisement {
            advbase: 1,
            advskew: 0,
            demote: 2,
        });
        assert_eq!(decision, Decision::BecomeMaster);
    }
}

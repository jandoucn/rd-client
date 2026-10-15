use super::recovery::ServiceState;

pub struct BootstrapObservation {
    pub agent: ServiceState,
    pub trusted_agent: bool,
    pub disabled_start: bool,
    pub agent_locally_disabled: bool,
    pub maintenance: bool,
    pub retry_due: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BootstrapAction {
    Observe,
    StartAgent,
    RecreateAgent,
}

pub fn decide_missing(o: &BootstrapObservation, intent_enabled: bool) -> BootstrapAction {
    if o.agent == ServiceState::Absent
        && o.trusted_agent
        && !o.disabled_start
        && !o.agent_locally_disabled
        && !o.maintenance
        && o.retry_due
        && intent_enabled
    {
        BootstrapAction::RecreateAgent
    } else {
        BootstrapAction::Observe
    }
}

pub fn decide(o: &BootstrapObservation) -> BootstrapAction {
    if o.agent == ServiceState::Stopped
        && o.trusted_agent
        && !o.disabled_start
        && !o.agent_locally_disabled
        && !o.maintenance
        && o.retry_due
    {
        BootstrapAction::StartAgent
    } else {
        BootstrapAction::Observe
    }
}

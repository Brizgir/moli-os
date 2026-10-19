//! What Moli proposes about an existing automation, for a person to
//! confirm: a new version, a pause, a resumption, a deletion, a run.
//!
//! An automation a person approved is the house's intent: nothing of it
//! changes on Moli's word alone. So Moli prepares, the proposal waits on
//! the screen (the card under its answer, the Automations page), and a
//! person confirms with the same gesture as approving an automation
//! (`asHuman`: the code when there is no session). Until then, nothing has
//! moved: the automation keeps running as it was. A proposal lives ten
//! minutes, like an approval request.

use std::time::Duration;

use moli_i18n::tr;
use serde::Serialize;

use crate::{
    Actor, Author, Automation, AutomationError, Automations, Level, Run, Version, lock, tidy,
    tidy_name,
};

/// What the proposal would do once confirmed.
#[derive(Clone, Debug)]
pub enum Intent {
    /// Replace the automation with this version and approve it.
    Update(Box<Automation>),
    /// Switch it off.
    Pause,
    /// Approve the current version again (switch it back on).
    Resume,
    Delete,
    /// Run it now, for real.
    Run,
}

impl Intent {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Update(_) => "update",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Delete => "delete",
            Self::Run => "run",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Proposal {
    pub id: u64,
    /// The automation it is about.
    pub automation: String,
    /// Its name (the new one, for an update).
    pub name: String,
    pub kind: &'static str,
    /// What it does once confirmed, in the house's words.
    pub summary: String,
    /// Protected rooms the (new) version reaches.
    pub protected: Vec<String>,
    /// The new version, for an update (what a person confirms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    pub proposed_by: Author,
    pub created: u64,
    pub expires: u64,
    #[serde(skip)]
    intent: Intent,
}

/// What a confirmation did.
#[derive(Debug)]
pub enum Confirmed {
    /// The automation as it is now.
    Automation(Automation),
    /// Running.
    Run(Run),
    Deleted,
}

/// How long a proposal waits for a person.
pub const PROPOSAL_TTL: Duration = Duration::from_secs(10 * 60);
/// So that a chatty agent cannot pile them up.
const MAX_PROPOSALS: usize = 20;

impl Automations {
    /// Moli (or an agent) proposes something about `target`. Nothing
    /// changes: the proposal waits for a person. An update's version is
    /// bounded and checked like a saved one (errors refuse it). A new
    /// proposal about the same automation replaces the previous one.
    ///
    /// # Errors
    /// `Unknown` target; `Invalid` version or state (resuming what already
    /// runs, pausing what is off, running a draft).
    pub fn propose(
        &self,
        target: &str,
        intent: Intent,
        by: Actor,
    ) -> Result<Proposal, AutomationError> {
        let current = self.get(target).ok_or(AutomationError::Unknown)?;
        let intent = match intent {
            Intent::Update(mut a) => {
                a.id.clone_from(&current.id);
                if a.name.trim().is_empty() {
                    a.name.clone_from(&current.name);
                }
                a.name = tidy_name(&a.name);
                tidy(&mut a.graph)?;
                let check = self.check(&a.graph);
                if !check.ok() {
                    let why: Vec<&str> = check
                        .problems
                        .iter()
                        .filter(|p| p.level == Level::Error)
                        .map(|p| p.message.as_str())
                        .collect();
                    return Err(AutomationError::Invalid(why.join(" ; ")));
                }
                Intent::Update(a)
            }
            Intent::Resume => {
                if current.is_live() {
                    return Err(AutomationError::Invalid(tr!(
                        "moteur.proposition.deja_actif"
                    )));
                }
                if !self.check(&current.graph).ok() {
                    return Err(AutomationError::Invalid(tr!(
                        "moteur.proposition.a_corriger"
                    )));
                }
                Intent::Resume
            }
            Intent::Pause => {
                if !current.enabled {
                    return Err(AutomationError::Invalid(tr!(
                        "moteur.proposition.deja_coupe"
                    )));
                }
                Intent::Pause
            }
            Intent::Run => {
                if !current.is_live() {
                    return Err(AutomationError::Invalid(tr!(
                        "moteur.proposition.pas_valide"
                    )));
                }
                Intent::Run
            }
            Intent::Delete => Intent::Delete,
        };
        let (name, check) = match &intent {
            Intent::Update(a) => (a.name.clone(), self.check(&a.graph)),
            _ => (current.name.clone(), self.check(&current.graph)),
        };
        let now = moli_core::now_ms();
        let proposal = Proposal {
            id: self.next_id(),
            automation: current.id.clone(),
            name,
            kind: intent.kind(),
            summary: check.summary,
            protected: check.protected,
            version: match &intent {
                Intent::Update(a) => Some(Version {
                    mode: a.mode,
                    graph: a.graph.clone(),
                }),
                _ => None,
            },
            proposed_by: by.author,
            created: now,
            expires: now + u64::try_from(PROPOSAL_TTL.as_millis()).unwrap_or(u64::MAX),
            intent,
        };
        {
            let mut list = lock(&self.0.proposals);
            list.retain(|p| p.expires > now && p.automation != proposal.automation);
            while list.len() >= MAX_PROPOSALS {
                list.remove(0);
            }
            list.push(proposal.clone());
        }
        tracing::info!(
            automation = %proposal.automation,
            kind = proposal.kind,
            who = %by.who(),
            "automation change proposed"
        );
        self.changed(&proposal.automation);
        Ok(proposal)
    }

    /// The proposals waiting for a person, oldest first.
    pub fn proposals(&self) -> Vec<Proposal> {
        let now = moli_core::now_ms();
        let mut list = lock(&self.0.proposals);
        list.retain(|p| p.expires > now);
        list.clone()
    }

    /// The proposal about one automation, if any.
    pub fn proposal_for(&self, automation: &str) -> Option<Proposal> {
        self.proposals()
            .into_iter()
            .find(|p| p.automation == automation)
    }

    /// A person confirms: the proposal is carried out in their name, by the
    /// same paths as the dashboard's buttons (so journaled as theirs).
    ///
    /// # Errors
    /// `NeedsHuman` for anyone else; `Unknown` once gone or expired; what
    /// the change itself refuses.
    pub async fn confirm(&self, id: u64, by: Actor) -> Result<Confirmed, AutomationError> {
        if !by.human {
            return Err(AutomationError::NeedsHuman);
        }
        let proposal = self.take_proposal(id)?;
        let target = proposal.automation.as_str();
        let done = match proposal.intent {
            Intent::Update(a) => {
                let saved = self.save(*a, by).await?;
                let fingerprint = saved.fingerprint();
                Confirmed::Automation(self.approve(target, &fingerprint, by).await?)
            }
            Intent::Pause => {
                Confirmed::Automation(self.set_enabled(target, false, None, by).await?)
            }
            Intent::Resume => {
                let current = self.get(target).ok_or(AutomationError::Unknown)?;
                Confirmed::Automation(self.approve(target, &current.fingerprint(), by).await?)
            }
            Intent::Delete => {
                self.delete(target, by).await?;
                Confirmed::Deleted
            }
            Intent::Run => {
                let current = self.get(target).ok_or(AutomationError::Unknown)?;
                Confirmed::Run(
                    self.run_now(target, false, Some(&current.fingerprint()), by)
                        .await?,
                )
            }
        };
        self.changed(target);
        Ok(done)
    }

    /// Anyone may say no: refusing never moves anything.
    ///
    /// # Errors
    /// `Unknown` once gone or expired.
    pub fn dismiss(&self, id: u64) -> Result<(), AutomationError> {
        let proposal = self.take_proposal(id)?;
        self.changed(&proposal.automation);
        Ok(())
    }

    fn take_proposal(&self, id: u64) -> Result<Proposal, AutomationError> {
        let now = moli_core::now_ms();
        let mut list = lock(&self.0.proposals);
        list.retain(|p| p.expires > now);
        let i = list
            .iter()
            .position(|p| p.id == id)
            .ok_or(AutomationError::Unknown)?;
        Ok(list.remove(i))
    }

    /// Forgets the proposals about `automation`: someone changed or deleted
    /// it in the meantime, what Moli proposed no longer applies.
    pub(crate) fn drop_proposals(&self, automation: &str) {
        lock(&self.0.proposals).retain(|p| p.automation != automation);
    }
}

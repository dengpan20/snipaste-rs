//! Permission guidance state; authorization itself is always checked by macOS.

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Capture,
    ShowGuidance,
    WaitForPermission,
}

#[derive(Default)]
pub struct Gate {
    guidance_shown: bool,
}

impl Gate {
    pub fn check(&mut self, granted: bool) -> Decision {
        if granted {
            self.guidance_shown = false;
            Decision::Capture
        } else if !std::mem::replace(&mut self.guidance_shown, true) {
            Decision::ShowGuidance
        } else {
            Decision::WaitForPermission
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_denials_show_guidance_once_and_never_allow_capture() {
        let mut gate = Gate::default();
        assert_eq!(gate.check(false), Decision::ShowGuidance);
        for _ in 0..10 {
            assert_eq!(gate.check(false), Decision::WaitForPermission);
        }
    }

    #[test]
    fn a_grant_after_guidance_resumes_capture() {
        let mut gate = Gate::default();
        assert_eq!(gate.check(false), Decision::ShowGuidance);
        assert_eq!(gate.check(true), Decision::Capture);
        assert_eq!(gate.check(true), Decision::Capture);
    }

    #[test]
    fn a_previous_grant_does_not_bypass_revocation() {
        let mut gate = Gate::default();
        assert_eq!(gate.check(true), Decision::Capture);
        assert_eq!(gate.check(false), Decision::ShowGuidance);
        assert_eq!(gate.check(false), Decision::WaitForPermission);
    }
}

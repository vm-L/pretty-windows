/// Normalized playback status across SMTC sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Other,
}

/// Minimal session metadata needed for arbitration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub status: PlaybackState,
    pub has_media_info: bool,
}

/// User selection policy for active SMTC media session.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SelectionMode {
    #[default]
    Auto,
    Manual(String),
}

/// Arbitrates which SMTC session is displayed and controlled by MediaBar.
#[derive(Clone, Debug, Default)]
pub struct SessionArbitrator {
    mode: SelectionMode,
    last_auto_id: Option<String>,
}

impl SessionArbitrator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> &SelectionMode {
        &self.mode
    }

    pub fn set_auto(&mut self) {
        self.mode = SelectionMode::Auto;
    }

    pub fn set_manual(&mut self, id: String) {
        self.mode = SelectionMode::Manual(id);
    }

    /// Selects the active session ID according to user policy and smart sticky rules:
    /// - If `Manual(id)` exists in `sessions`, always select it.
    /// - If `Manual(id)` disappears from `sessions`, fall back to `Auto`.
    /// - In `Auto`, prefer `Playing` sessions (sticking to `last_auto_id` or `system_current_id` if playing),
    ///   then stick to `last_auto_id` while paused so pausing doesn't unexpectedly jump sources,
    ///   otherwise pick `system_current_id` -> metadata-rich session -> first session.
    pub fn select(
        &mut self,
        sessions: &[SessionSummary],
        system_current_id: Option<&str>,
    ) -> Option<String> {
        if sessions.is_empty() {
            self.mode = SelectionMode::Auto;
            self.last_auto_id = None;
            return None;
        }

        if let SelectionMode::Manual(ref target_id) = self.mode {
            if let Some(found) = sessions.iter().find(|s| &s.id == target_id) {
                return Some(found.id.clone());
            }
            // Manual source closed/disappeared -> automatically fall back to Auto mode.
            self.mode = SelectionMode::Auto;
        }

        let selected = self.select_auto(sessions, system_current_id);
        self.last_auto_id = selected.clone();
        selected
    }

    fn select_auto(
        &self,
        sessions: &[SessionSummary],
        system_current_id: Option<&str>,
    ) -> Option<String> {
        let any_playing = sessions.iter().any(|s| s.status == PlaybackState::Playing);
        if any_playing {
            if let Some(ref last) = self.last_auto_id {
                if sessions
                    .iter()
                    .any(|s| &s.id == last && s.status == PlaybackState::Playing)
                {
                    return Some(last.clone());
                }
            }
            if let Some(sys) = system_current_id {
                if sessions
                    .iter()
                    .any(|s| s.id == sys && s.status == PlaybackState::Playing)
                {
                    return Some(sys.to_string());
                }
            }
            if let Some(playing) = sessions.iter().find(|s| s.status == PlaybackState::Playing) {
                return Some(playing.id.clone());
            }
        }

        if let Some(ref last) = self.last_auto_id {
            if sessions.iter().any(|s| &s.id == last) {
                return Some(last.clone());
            }
        }

        if let Some(sys) = system_current_id {
            if sessions.iter().any(|s| s.id == sys) {
                return Some(sys.to_string());
            }
        }

        sessions
            .iter()
            .find(|s| s.has_media_info)
            .or_else(|| sessions.first())
            .map(|s| s.id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, status: PlaybackState) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            status,
            has_media_info: true,
        }
    }

    #[test]
    fn auto_prefers_playing_and_sticks_when_paused() {
        let mut arb = SessionArbitrator::new();
        let list = vec![
            session("chrome", PlaybackState::Paused),
            session("spotify", PlaybackState::Playing),
        ];
        assert_eq!(arb.select(&list, Some("chrome")), Some("spotify".to_string()));

        // User pauses Spotify -> stays on Spotify rather than jumping to paused Chrome
        let paused_list = vec![
            session("chrome", PlaybackState::Paused),
            session("spotify", PlaybackState::Paused),
        ];
        assert_eq!(
            arb.select(&paused_list, Some("chrome")),
            Some("spotify".to_string())
        );

        // Chrome starts playing -> switches to Chrome
        let chrome_plays = vec![
            session("chrome", PlaybackState::Playing),
            session("spotify", PlaybackState::Paused),
        ];
        assert_eq!(
            arb.select(&chrome_plays, Some("spotify")),
            Some("chrome".to_string())
        );
    }

    #[test]
    fn manual_mode_locks_and_reverts_when_missing() {
        let mut arb = SessionArbitrator::new();
        arb.set_manual("cloudmusic".to_string());

        let list = vec![
            session("edge", PlaybackState::Playing),
            session("cloudmusic", PlaybackState::Paused),
        ];
        assert_eq!(
            arb.select(&list, Some("edge")),
            Some("cloudmusic".to_string())
        );
        assert_eq!(
            arb.mode(),
            &SelectionMode::Manual("cloudmusic".to_string())
        );

        // Manual source exits -> automatically falls back to Auto and selects Playing Edge
        let without_manual = vec![session("edge", PlaybackState::Playing)];
        assert_eq!(
            arb.select(&without_manual, Some("edge")),
            Some("edge".to_string())
        );
        assert_eq!(arb.mode(), &SelectionMode::Auto);
    }
}

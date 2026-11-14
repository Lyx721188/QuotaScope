use super::*;

impl SettingsApp {
    pub(super) fn notices_view(&self, context: &ViewContext<Self>) -> View {
        use quotascope_core::localization::t;
        let mut rows = Vec::<View>::new();
        if self.star_banner {
            rows.push(self.surface(StackPanel::new().spacing(8.0).children((
                self.muted(t("You have used QuotaScope for two weeks. A star on GitHub helps others find the project.")),
                row((
                    Button::new().on_click(context.message(Message::OpenGitHub)).content(t("Star on GitHub")),
                    Button::new().on_click(context.message(Message::DismissStar)).content(t("Close")),
                )),
            ))));
        }
        if let Some(message) = quotascope_core::notices::message() {
            rows.push(
                self.surface(
                    StackPanel::new().spacing(8.0).children((
                        section("A message from the QuotaScope maintainer"),
                        TextBlock::new()
                            .text(message)
                            .text_wrapping(TextWrapping::Wrap)
                            .is_text_selection_enabled(true),
                        Button::new()
                            .on_click(context.message(Message::DismissNotice))
                            .content(t("Don’t show again for this version")),
                    )),
                ),
            );
        }
        StackPanel::new()
            .spacing(12.0)
            .children((View::keyed_fragment(rows.into_iter().enumerate()),))
    }
}

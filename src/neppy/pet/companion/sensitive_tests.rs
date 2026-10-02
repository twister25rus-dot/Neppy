use proptest::prelude::*;

use super::*;

fn sel(raw: &str) -> ScrubResult {
    scrub(raw, &ScrubCtx::selection(false))
}
fn clip(raw: &str) -> ScrubResult {
    scrub(raw, &ScrubCtx::clipboard(false))
}
fn ocr(raw: &str) -> ScrubResult {
    scrub(raw, &ScrubCtx::ocr())
}

/// Secrets are assembled at runtime so no literal credential sits in the repo.
fn anthropic_key() -> String {
    format!("sk-ant-{}", "a1B2c3D4".repeat(5))
}
fn github_token() -> String {
    format!("ghp_{}", "AbCd1234".repeat(4))
}
fn aws_key() -> String {
    format!("AKIA{}", "ABCD1234EFGH5678")
}
fn jwt() -> String {
    format!(
        "eyJ{}.eyJ{}.{}",
        "hbGciOiJIUzI1", "zdWIiOiIxMjM0NTY3", "SflKxwRJSMeKKF2QT4fw"
    )
}
const PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\nKgwggSkAgEAAoIBAQC7\n-----END PRIVATE KEY-----";
const SEED: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

fn assert_redacted_without(r: ScrubResult, needle: &str) {
    match r {
        ScrubResult::Redacted(s, kinds) => {
            assert!(
                !s.as_str().contains(needle),
                "leaked {needle} in {:?}",
                s.as_str()
            );
            assert!(!kinds.is_empty());
        }
        other => panic!("expected Redacted, got {other:?}"),
    }
}

#[test]
fn card_number_is_redacted() {
    assert_redacted_without(
        sel("Please charge card 4111 1111 1111 1111 for the order today."),
        "4111",
    );
}

#[test]
fn fullwidth_card_digits_are_redacted() {
    let fw = "４１１１ １１１１ １１１１ １１１１";
    assert_redacted_without(
        sel(&format!("Please charge card {fw} for the order today.")),
        "４１１１",
    );
}

#[test]
fn api_keys_and_tokens_are_redacted() {
    for secret in [anthropic_key(), github_token(), aws_key(), jwt()] {
        assert_redacted_without(
            sel(&format!("export it now: {secret} and then deploy the app")),
            &secret,
        );
    }
}

#[test]
fn pem_blocks_are_dropped_everywhere() {
    for ctx in [
        ScrubCtx::title(),
        ScrubCtx::selection(false),
        ScrubCtx::clipboard(false),
        ScrubCtx::ocr(),
        ScrubCtx::generated(),
    ] {
        assert_eq!(
            scrub(PEM, &ctx),
            ScrubResult::Drop(SensitiveKind::PemBlock),
            "{ctx:?}"
        );
    }
    // A truncated block (no END line) is dropped too.
    assert_eq!(
        sel("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk"),
        ScrubResult::Drop(SensitiveKind::PemBlock)
    );
}

#[test]
fn otp_codes_are_redacted() {
    assert_redacted_without(
        sel("Your verification code is 482913. Do not share it."),
        "482913",
    );
    assert_redacted_without(sel("one-time passcode: 1234 expires soon"), "1234");
}

#[test]
fn bare_six_digit_clipboard_is_dropped_as_otp() {
    // Redacted to a lone marker, which is more than half redaction: dropped.
    assert!(clip("482913").drop_kind().is_some());
}

#[test]
fn password_lines_are_redacted() {
    assert_redacted_without(
        sel("My login is bob and the password: hunter2! for the portal today"),
        "hunter2",
    );
    assert!(sel("password: hunter2!")
        .text()
        .is_none_or(|t| !t.as_str().contains("hunter2")));
}

#[test]
fn seed_phrase_is_dropped() {
    assert_eq!(sel(SEED), ScrubResult::Drop(SensitiveKind::SeedPhrase));
    assert_eq!(clip(SEED), ScrubResult::Drop(SensitiveKind::SeedPhrase));
}

#[test]
fn password_like_token_from_clipboard_is_dropped() {
    assert_eq!(
        clip("Tr0ub4dor&3xK!"),
        ScrubResult::Drop(SensitiveKind::PasswordToken)
    );
    assert_eq!(
        sel("Tr0ub4dor&3xK!"),
        ScrubResult::Drop(SensitiveKind::PasswordToken)
    );
}

#[test]
fn identifiers_urls_and_paths_are_not_password_like() {
    for t in [
        "https://example.com/a/b?c=1",
        "/Users/alex/Neppy/src/main.rs",
        "my_variable_Name2",
        "src/neppy/pet/mod.rs",
        "alex@example.com",
        "lowercaseonly",
    ] {
        assert!(
            clip(t).drop_kind() != Some(SensitiveKind::PasswordToken),
            "{t} must not be treated as a password"
        );
    }
}

#[test]
fn secure_field_and_concealed_clipboard_drop_everything() {
    assert_eq!(
        scrub("anything", &ScrubCtx::selection(true)),
        ScrubResult::Drop(SensitiveKind::SecureField)
    );
    assert_eq!(
        scrub("anything", &ScrubCtx::clipboard(true)),
        ScrubResult::Drop(SensitiveKind::ConcealedClipboard)
    );
    assert_eq!(
        SensitiveKind::SecureField.drop_reason(),
        DropReason::SecureField
    );
    assert_eq!(
        SensitiveKind::ConcealedClipboard.drop_reason(),
        DropReason::ConcealedClipboard
    );
    assert_eq!(
        SensitiveKind::PemBlock.drop_reason(),
        DropReason::SensitiveContent
    );
}

#[test]
fn mostly_redacted_text_is_dropped() {
    let r = sel(&format!("{} {}", anthropic_key(), github_token()));
    assert_eq!(r, ScrubResult::Drop(SensitiveKind::TooRedacted));
}

#[test]
fn plain_prose_is_clean_even_with_a_year() {
    for t in [
        "In 2024 the team shipped the new release, and users loved it.",
        "The meeting moved to Thursday. Please bring the budget draft.",
        "Quarterly planning - Google Docs",
    ] {
        match sel(t) {
            ScrubResult::Clean(s) => assert_eq!(s.as_str(), t),
            other => panic!("false positive on {t:?}: {other:?}"),
        }
    }
}

// ---- OCR (multi-line screen text, §5 D2) ----

#[test]
fn ocr_with_a_card_number_is_redacted_line_wise() {
    let screen = "Order summary\nCard: 4111 1111 1111 1111\nExpires 12/29\nThanks for shopping";
    match ocr(screen) {
        ScrubResult::Redacted(s, kinds) => {
            assert!(!s.as_str().contains("4111"));
            assert!(s.as_str().contains("Order summary"));
            assert!(s.as_str().contains("Thanks for shopping"));
            assert!(kinds.contains(&SensitiveKind::Pii));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ocr_with_an_otp_is_redacted() {
    let screen = "Messages\nBank: Your verification code is 482913\nDo not share it with anyone";
    match ocr(screen) {
        ScrubResult::Redacted(s, kinds) => {
            assert!(!s.as_str().contains("482913"));
            assert!(kinds.contains(&SensitiveKind::Otp));
            assert!(s.as_str().contains("Messages"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ocr_with_an_api_key_is_redacted() {
    let screen = format!(
        "Settings\nANTHROPIC_API_KEY={}\nSave changes\nCancel",
        anthropic_key()
    );
    match ocr(&screen) {
        ScrubResult::Redacted(s, kinds) => {
            assert!(!s.as_str().contains("sk-ant"));
            assert!(kinds.contains(&SensitiveKind::Secret));
            assert!(s.as_str().contains("Save changes"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ocr_with_a_pem_block_drops_the_whole_capture() {
    let screen = format!("Terminal\n$ cat id_ed25519\n{PEM}\n$ ");
    assert_eq!(ocr(&screen), ScrubResult::Drop(SensitiveKind::PemBlock));
}

#[test]
fn ocr_with_a_numbered_seed_grid_drops_the_whole_capture() {
    let screen = "Recovery phrase\n1 legal 2 winner 3 thank 4 year\n5 wave 6 sausage 7 worth 8 useful\n9 legal 10 winner 11 thank 12 yellow\nCopy to clipboard";
    assert_eq!(ocr(screen), ScrubResult::Drop(SensitiveKind::SeedPhrase));
}

#[test]
fn ocr_with_a_password_line_is_redacted() {
    let screen =
        "Sign in to the portal\nUsername: bob\nPassword: hunter2!\nForgot your password?\nLogin";
    match ocr(screen) {
        ScrubResult::Redacted(s, _) => {
            assert!(!s.as_str().contains("hunter2"));
            assert!(s.as_str().contains("Sign in to the portal"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ocr_of_ordinary_screen_text_is_clean() {
    let screen = "error[E0308]: mismatched types\n --> src/main.rs:4:5\n  |\n4 |     let x: u32 = \"a\";\n  |                  ^^^ expected `u32`, found `&str`";
    match ocr(screen) {
        ScrubResult::Clean(s) => assert!(s.as_str().contains("E0308")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn ocr_text_is_capped_without_cutting_a_secret_in_half() {
    let screen = format!("{}{} tail", "Word, ".repeat(790), anthropic_key());
    let s = ocr(&screen).into_text().unwrap();
    assert!(s.as_str().chars().count() <= MAX_OCR_CHARS);
    assert!(!s.as_str().contains("sk-ant"), "partial key survived");
    // A long clean text is capped with an ellipsis and stays within the cap.
    let long = "plain words only. ".repeat(1000);
    let c = ocr(&long).into_text().unwrap();
    assert!(c.as_str().chars().count() <= MAX_OCR_CHARS);
    assert!(c.as_str().ends_with('…'));
}

#[test]
fn long_lowercase_listings_are_dropped_as_possible_seeds() {
    // Documented false-positive direction: 12+ plain lowercase words in a row.
    let listing = "word ".repeat(40);
    assert_eq!(ocr(&listing), ScrubResult::Drop(SensitiveKind::SeedPhrase));
    // Punctuation, capitals or short words break the run.
    assert!(
        ocr("the cat sat on a mat and then it went to bed early, no fuss at all")
            .text()
            .is_some()
    );
}

#[test]
fn zero_width_characters_do_not_hide_a_key() {
    let key = anthropic_key();
    let split = format!("{}\u{200b}{}", &key[..10], &key[10..]);
    assert_redacted_without(
        sel(&format!("here is a key {split} for later use ok")),
        &key[..10],
    );
}

#[test]
fn caps_per_source() {
    let t = format!("{} end", "Title, words. ".repeat(100));
    assert!(
        scrub(&t, &ScrubCtx::title())
            .into_text()
            .unwrap()
            .as_str()
            .chars()
            .count()
            <= MAX_TITLE_CHARS
    );
    assert!(
        scrub(&t, &ScrubCtx::selection(false).capped(50))
            .into_text()
            .unwrap()
            .as_str()
            .chars()
            .count()
            <= 50
    );
}

#[test]
fn table_no_fixture_secret_survives_any_source() {
    let secrets = [anthropic_key(), github_token(), aws_key(), jwt()];
    for secret in &secrets {
        for src in [
            ScrubSource::Title,
            ScrubSource::Selection,
            ScrubSource::Clipboard,
            ScrubSource::Ocr,
            ScrubSource::Generated,
        ] {
            let raw = format!("log line before\nvalue is {secret} and more words after\nlast line");
            if let Some(t) = scrub(&raw, &ScrubCtx::new(src)).text() {
                assert!(!t.as_str().contains(secret.as_str()), "{src:?} leaked");
            }
        }
    }
}

fn assert_idempotent(raw: &str) {
    for src in [
        ScrubSource::Title,
        ScrubSource::Selection,
        ScrubSource::Clipboard,
        ScrubSource::Ocr,
        ScrubSource::Generated,
    ] {
        let ctx = ScrubCtx::new(src);
        if let Some(first) = scrub(raw, &ctx).into_text() {
            let again = scrub(first.as_str(), &ctx);
            let second = again.text().map(|s| s.as_str().to_string());
            assert_eq!(
                second.as_deref(),
                Some(first.as_str()),
                "not idempotent for {src:?}: {raw:?}"
            );
        }
    }
}

#[test]
fn scrub_is_idempotent_on_fixtures() {
    let fixtures = [
        "Your verification code is 482913 ok",
        "card 4111 1111 1111 1111 thanks for the order",
        "the password: hunter2! for the portal is saved",
        "Order summary\nCard: 4111 1111 1111 1111\nExpires 12/29\nThanks",
        &format!("key {} then {}", anthropic_key(), github_token()),
        &"Word, ".repeat(2500),
        "plain prose with a year 2024.",
    ];
    for f in fixtures {
        assert_idempotent(f);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn scrub_is_idempotent_on_random_text(raw in "[ -~\n]{0,300}") {
        assert_idempotent(&raw);
    }

    #[test]
    fn injected_secrets_never_survive(
        before in "[a-zA-Z ,.]{0,60}",
        after in "[a-zA-Z ,.]{0,60}",
        pick in 0usize..4,
    ) {
        let secrets = [anthropic_key(), github_token(), aws_key(), jwt()];
        let secret = &secrets[pick];
        let raw = format!("{before}\n{secret}\n{after}");
        for src in [ScrubSource::Selection, ScrubSource::Clipboard, ScrubSource::Ocr] {
            if let Some(t) = scrub(&raw, &ScrubCtx::new(src)).text() {
                prop_assert!(!t.as_str().contains(secret.as_str()));
            }
        }
    }
}

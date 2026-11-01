/*
 * Copyright (C) 2026 MysticFox / Pranav Karande (pranav158/Mystic-Paradox)
 * Licensed under the GNU Affero General Public License v3.0.
 *
 * SPDX-License-Identifier: AGPL-3.0-only
 * Additional terms under AGPLv3 Section 7 apply. See ADDITIONAL_TERMS.md.
 */

// [2026-10-15] RUSTIC_ISLES_EMAIL_VERIFICATION: the mode (off by default, an unknown value is required), the one startup line, and the
// refusal to start with required and no working SMTP settings.
import assert from "node:assert/strict";
import test from "node:test";
import {
    AssertEmailVerificationConfig,
    EmailVerificationRequired,
    LogEmailVerificationPolicy,
    ResolveEmailVerification
} from "./emailVerification";

const SMTP = {
    RUSTIC_ISLES_SMTP_HOST: "smtp.example.test",
    RUSTIC_ISLES_SMTP_USER: "postmaster",
    RUSTIC_ISLES_SMTP_PASSWORD: "fixture-password"
};

test("unset or blank is off, the default; off is off; required is required, case-insensitively and trimmed", () => {
    assert.deepEqual(ResolveEmailVerification({}), { mode: "off", isDefault: true });
    assert.deepEqual(ResolveEmailVerification({ RUSTIC_ISLES_EMAIL_VERIFICATION: "  " }), { mode: "off", isDefault: true });
    assert.deepEqual(ResolveEmailVerification({ RUSTIC_ISLES_EMAIL_VERIFICATION: "OFF" }), { mode: "off", isDefault: false });
    assert.deepEqual(ResolveEmailVerification({ RUSTIC_ISLES_EMAIL_VERIFICATION: " Required " }), { mode: "required", isDefault: false });
});

test("an unrecognised value is required, with a refusal that does not repeat the value", () => {
    for (const value of ["on", "true", "maybe", "requried"]) {
        const Policy = ResolveEmailVerification({ RUSTIC_ISLES_EMAIL_VERIFICATION: value });
        assert.equal(Policy.mode, "required", value);
        assert.match(Policy.refusal ?? "", /must be required or off/, value);
        assert.doesNotMatch(Policy.refusal ?? "", new RegExp(value), value);
    }
    assert.equal(EmailVerificationRequired({ RUSTIC_ISLES_EMAIL_VERIFICATION: "on" }), true);
    assert.equal(EmailVerificationRequired({}), false);
});

test("startup: one line for the mode; a warning first for an unrecognised value (logged once, at startup)", () => {
    const Lines: Array<{ level: string; message: string }> = [];
    const Sink = {
        info: (message: string) => { Lines.push({ level: "info", message }); },
        warn: (message: string) => { Lines.push({ level: "warn", message }); }
    };
    LogEmailVerificationPolicy(ResolveEmailVerification({ RUSTIC_ISLES_EMAIL_VERIFICATION: "required" }), Sink);
    assert.deepEqual(Lines.map((line) => line.level), ["info"]);
    assert.match(Lines[0]!.message, /RUSTIC_ISLES_EMAIL_VERIFICATION=required: sign-in needs a verified email \(launcher 0\.1\.67/);

    Lines.length = 0;
    LogEmailVerificationPolicy(ResolveEmailVerification({ RUSTIC_ISLES_EMAIL_VERIFICATION: "maybe" }), Sink);
    assert.deepEqual(Lines.map((line) => line.level), ["warn", "info"]);

    Lines.length = 0;
    LogEmailVerificationPolicy(ResolveEmailVerification({}), Sink);
    assert.match(Lines[0]!.message, /not set \(default off\)/);
});

test("required without working SMTP settings stops the start, and the error names the keys but no value", () => {
    const Environment = { RUSTIC_ISLES_EMAIL_VERIFICATION: "required", RUSTIC_ISLES_SMTP_USER: "postmaster" };
    assert.throws(() => AssertEmailVerificationConfig(Environment), (error: unknown) => {
        const Message = String((error as Error).message);
        return /RUSTIC_ISLES_EMAIL_VERIFICATION=required needs working SMTP settings/.test(Message)
            && /RUSTIC_ISLES_SMTP_HOST/.test(Message)
            && /RUSTIC_ISLES_SMTP_PASSWORD/.test(Message)
            && !/postmaster/.test(Message);
    });
    assert.throws(() => AssertEmailVerificationConfig({ RUSTIC_ISLES_EMAIL_VERIFICATION: "required", ...SMTP, RUSTIC_ISLES_SMTP_SECURE: "ssl" }),
        /RUSTIC_ISLES_SMTP_SECURE/);
});

test("required with complete SMTP settings starts; off and unset never check the SMTP settings", () => {
    assert.doesNotThrow(() => AssertEmailVerificationConfig({ RUSTIC_ISLES_EMAIL_VERIFICATION: "required", ...SMTP }));
    assert.doesNotThrow(() => AssertEmailVerificationConfig({ RUSTIC_ISLES_EMAIL_VERIFICATION: "off" }));
    assert.doesNotThrow(() => AssertEmailVerificationConfig({}));
});

import assert from "node:assert/strict";
import test from "node:test";
import { AssertRuntimeSafety } from "./runtimeSafety";

test("production rejects development authentication and raw capture", () => {
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "production", AUTH_MODE: "NONE",
        REALTIME_XMPP_DEV_CITY_MUC: "false", RUSTIC_ISLES_SERVICE_ROLE: "api", RUSTIC_ISLES_METRICS_TOKEN: "metrics-test-token" }), /AUTH_MODE=NONE/);
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "production", RUSTIC_ISLES_BODY_CAPTURE: "yes",
        REALTIME_XMPP_DEV_CITY_MUC: "false", RUSTIC_ISLES_SERVICE_ROLE: "api", RUSTIC_ISLES_METRICS_TOKEN: "metrics-test-token" }), /RUSTIC_ISLES_BODY_CAPTURE/);
});

test("a key with the old MYSTPAX_ prefix stops startup in every environment", () => {
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "development", MYSTPAX_LOG_ORIGIN_BODIES: "1" }),
        /RUSTIC_ISLES_ prefix: MYSTPAX_LOG_ORIGIN_BODIES/);
    assert.doesNotThrow(() => AssertRuntimeSafety({ NODE_ENV: "development", RUSTIC_ISLES_LOG_ORIGIN_BODIES: "1" }));
});

test("production requires the unauthorised City room fallback to be disabled", () => {
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "production", RUSTIC_ISLES_SERVICE_ROLE: "api" }), /REALTIME_XMPP_DEV_CITY_MUC/);
    assert.doesNotThrow(() => AssertRuntimeSafety({ NODE_ENV: "production", AUTH_MODE: "APIKEY",
        REALTIME_XMPP_DEV_CITY_MUC: "false", RUSTIC_ISLES_SERVICE_ROLE: "api", RUSTIC_ISLES_METRICS_TOKEN: "metrics-test-token",
        MONGODB_URI: "mongodb://guardit.example.test/?replicaSet=guardit" }));
});

test("production requires an explicit API or worker role", () => {
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "production", AUTH_MODE: "APIKEY",
        REALTIME_XMPP_DEV_CITY_MUC: "false", RUSTIC_ISLES_METRICS_TOKEN: "metrics-test-token" }), /RUSTIC_ISLES_SERVICE_ROLE/);
});

test("production API requires an authenticated metrics endpoint", () => {
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "production", AUTH_MODE: "APIKEY",
        REALTIME_XMPP_DEV_CITY_MUC: "false", RUSTIC_ISLES_SERVICE_ROLE: "api" }), /RUSTIC_ISLES_METRICS_TOKEN/);
});

test("production requires Mongo URI and bounded Mongo runtime settings", () => {
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "production", AUTH_MODE: "APIKEY",
        REALTIME_XMPP_DEV_CITY_MUC: "false", RUSTIC_ISLES_SERVICE_ROLE: "api",
        RUSTIC_ISLES_METRICS_TOKEN: "metrics-test-token" }), /MONGODB_URI/);
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "production", AUTH_MODE: "APIKEY",
        REALTIME_XMPP_DEV_CITY_MUC: "false", RUSTIC_ISLES_SERVICE_ROLE: "api",
        RUSTIC_ISLES_METRICS_TOKEN: "metrics-test-token",
        MONGODB_URI: "mongodb://guardit.example.test/?replicaSet=guardit",
        MONGODB_MAX_POOL_SIZE: "0" }), /MONGODB_MAX_POOL_SIZE/);
});

test("production rejects the combined API and worker process", () => {
    assert.throws(() => AssertRuntimeSafety({ NODE_ENV: "production", AUTH_MODE: "APIKEY",
        REALTIME_XMPP_DEV_CITY_MUC: "false", RUSTIC_ISLES_SERVICE_ROLE: "combined", RUSTIC_ISLES_METRICS_TOKEN: "metrics-test-token" }), /combined is local-development only/);
});

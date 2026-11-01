import { MetricsSection } from "../../observability/metrics";

// Gauges of the player-hosted hunt module that the metrics page always shows: the age of the newest
// hunt Guard evidence and whether central Tier 1 grants are enabled.
export const P2PMetricsSection: MetricsSection = {
    hiddenGauges: ["rustic_isles_reward_rollout_enabled", "rustic_isles_guard_last_received_timestamp_seconds"],
    render(now, readGauge) {
        const lastEvidence = readGauge("rustic_isles_guard_last_received_timestamp_seconds");
        return [
            { name: "rustic_isles_guard_evidence_age_seconds", help: "Age of the most recently received Guard evidence.",
                value: lastEvidence == undefined ? 0 : Math.max(0, now / 1000 - lastEvidence) },
            { name: "rustic_isles_reward_rollout_enabled", help: "Whether central Tier 1 grants are enabled.",
                value: readGauge("rustic_isles_reward_rollout_enabled") ?? 0 }
        ];
    }
};

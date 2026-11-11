#!/usr/bin/env bash
# dainty-test [suite...]: the server suites inside the test image, against what it was built with. The only suite is
# rust (the default). Exits 1 when any suite failed.
set -uo pipefail
export NO_COLOR=1

suite_rust() {
    if /usr/sbin/nft list tables >/dev/null 2>&1; then
        export DAINTY_TEST_FIREWALL=1
        echo "(firewall test: on, this container has CAP_NET_ADMIN)"
    else
        echo "(firewall test: off; run with --cap-add NET_ADMIN to include it)"
    fi
    # The list the image pre-builds (docker/Dockerfile's test stage)
    cd /opt/src && cargo test --locked --no-fail-fast -p dainty-protocol -p dainty-proxy -p dainty-remote \
        -p dainty-service-kit -p dainty-store -p dainty-gateway -p dainty-content -p dainty-admin -p dainty-hostkit \
        -p dainty-host -p dainty-controller -p dainty-server
}

[[ $# -gt 0 ]] || set -- rust
results=()
failed=0
for suite in "$@"; do
    started=$SECONDS
    case "$suite" in
        rust) (suite_rust) ;;
        *)
            echo "unknown suite $suite (rust)" >&2
            (exit 2)
            ;;
    esac
    status=$?
    took=$((SECONDS - started))
    if [[ $status -eq 0 ]]; then
        results+=("$suite: passed ($took s)")
    else
        results+=("$suite: FAILED ($took s)")
        failed=1
    fi
done

echo
echo "========== summary"
printf '%s\n' "${results[@]}"
exit "$failed"

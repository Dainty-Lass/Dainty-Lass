#!/bin/bash
# Dainty Lass: the server crates' tests, run inside the test image (docker/Dockerfile, target test), which is the
# server image plus the Rust toolchain and the repository's sources at /opt/src: `dainty-test [rust]`.
# docker/run-tests.sh runs it, then the compose harnesses.
#
#   rust      cargo test for the server crates (protocol, proxy, remote, service-kit, store, gateway, content, admin,
#             hostkit, host, controller), every test binary even after a failure. With CAP_NET_ADMIN
#             (docker run --cap-add NET_ADMIN) the allowlist's firewall test applies its nftables rules for real
#             (DAINTY_TEST_FIREWALL=1), in this container's own network namespace
#
# The Node reference's suites are not here: the image has no Node (npm test in their folders, Node 22 in CI).
set -uo pipefail

suites=("$@")
[ ${#suites[@]} -eq 0 ] && suites=(rust)
export NO_COLOR=1

can_net_admin() {
	# nft has the capability as a file capability; it works only when the container's bounding set allows it
	/usr/sbin/nft list tables >/dev/null 2>&1
}

run_suite() {
	case "$1" in
	rust)
		if can_net_admin; then
			export DAINTY_TEST_FIREWALL=1
			echo "(firewall test: on, this container has CAP_NET_ADMIN)"
		else
			echo "(firewall test: off; run with --cap-add NET_ADMIN to include it)"
		fi
		cd /opt/src && cargo test --locked --no-fail-fast \
			-p dainty-protocol -p dainty-proxy -p dainty-remote -p dainty-service-kit -p dainty-store -p dainty-gateway -p dainty-content -p dainty-admin -p dainty-hostkit -p dainty-host \
			-p dainty-controller
		;;
	*)
		echo "unknown suite $1 (rust)" >&2
		return 2
		;;
	esac
}

declare -A result
failed=0
for suite in "${suites[@]}"; do
	printf '\n========== %s\n' "$suite"
	start=$SECONDS
	if (run_suite "$suite"); then
		result[$suite]="passed ($((SECONDS - start)) s)"
	else
		result[$suite]="FAILED ($((SECONDS - start)) s)"
		failed=1
	fi
done

printf '\n========== summary\n'
for suite in "${suites[@]}"; do
	printf '  %-9s %s\n' "$suite" "${result[$suite]}"
done
exit "$failed"

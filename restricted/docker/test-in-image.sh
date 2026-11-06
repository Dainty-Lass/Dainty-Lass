#!/bin/bash
# DaintyLass: every server test suite, run inside the test image (docker/Dockerfile, target test),
# which is the server image plus the Rust toolchain: `dainty-test [rust] [backend] [director]` (all three by
# default). docker/run-tests.sh runs it, then the compose smoke test; `docker compose run --rm test` runs it with
# the server's game-data volumes mounted, so the *.gamedata.test.ts files check your real data too.
#
#   rust      cargo test for the server crates (protocol, proxy, service-kit, store, gateway, content, admin, host,
#             controller, backend, director), every test binary even after a failure. With CAP_NET_ADMIN
#             (docker run --cap-add NET_ADMIN) the allowlist's nftables test applies its rules for real
#             (DAINTY_TEST_NFT=1), in this container's own network namespace
#   backend   the Node backend's tests (rustic-isles' ParadoxBackend, npm test)
#   director  the Node Director's tests (rustic-isles' ParadoxDirector, npm test)
set -uo pipefail

root=/opt/dainty-lass
suites=("$@")
[ ${#suites[@]} -eq 0 ] && suites=(rust)
export NPM_CONFIG_UPDATE_NOTIFIER=false NO_COLOR=1

can_net_admin() {
	# nft has the capability as a file capability; it works only when the container's bounding set allows it
	/usr/sbin/nft list tables >/dev/null 2>&1
}

run_suite() {
	case "$1" in
	rust)
		if can_net_admin; then
			export DAINTY_TEST_NFT=1
			echo "(nftables test: on, this container has CAP_NET_ADMIN)"
		else
			echo "(nftables test: off; run with --cap-add NET_ADMIN to include it)"
		fi
		cd "$root" && cargo test --locked --no-fail-fast \
			-p protocol -p proxy -p remote -p service-kit -p store -p gateway -p content -p admin -p hostkit -p host \
			-p controller
		;;
	backend) cd "$root/modules/rustic-isles/reference/1.14.7/ParadoxBackend" && npm test ;;
	director) cd "$root/modules/rustic-isles/reference/1.14.7/ParadoxDirector" && npm test ;;
	*)
		echo "unknown suite $1 (rust, backend, director)" >&2
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

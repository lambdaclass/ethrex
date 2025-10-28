#!/bin/bash

packages=(
    "cmd/ethrex"
    "crates/blockchain"
    "crates/blockchain/dev"
    "crates/common"
    "crates/common/rlp"
    "crates/common/trie"
    "crates/common/crypto"
    "crates/l2/"
    "crates/l2/common"
    "crates/l2/prover"
    "crates/l2/prover/src/guest_program"
    "crates/l2/sdk"
    "crates/l2/sdk/contract_utils"
    "crates/l2/storage"
    "crates/l2/networking/rpc"
    "crates/networking/p2p"
    "crates/networking/rpc"
    "crates/storage"
    "crates/vm"
    "crates/vm/levm"
    "crates/vm/levm/runner"
    "tooling/ef_tests/blockchain"
    "tooling/ef_tests/state_v2"
    "tooling/genesis"
    "tooling/hive_report"
    "tooling/load_test"
    "tooling/loc"
    "tooling/archive_sync"
    "crates/common/config"
    "tooling/reorgs"
)

special_packages=(
    "crates/blockchain"
    "crates/common"
    "crates/common/crypto"
    "crates/l2/"
    "crates/l2/prover"
    "crates/l2/prover/src/guest_program"
    "crates/l2/storage"
    "crates/networking/p2p"
    "crates/networking/rpc"
    "crates/vm"
    "crates/vm/levm"
    "tooling/ef_tests/blockchain"
)

l2_packages=(
    "crates/l2/"
    "crates/l2/prover"
    "crates/l2/prover/src/guest_program"
)

get_cargo_args() {
    local package="$1"
    local variant="$2"
    case "$variant" in
        default)
            echo ""
            ;;
        sp1)
            echo "-F sp1 --no-default-features"
            ;;
        risc0)
            echo "-F risc0 --no-default-features"
            ;;
        l2)
            echo "-F l2"
            ;;
        l2+sp1)
            echo "-F l2,sp1 --no-default-features"
            ;;
        l2+risc0)
            echo "-F l2,risc0 --no-default-features"
            ;;
        l1,risc0-guest)
            echo "-F l1,risc0-guest"
            ;;
        l1,sp1-guest)
            echo "-F l1,sp1-guest"
            ;;
        l2,sp1-guest)
            echo "-F l2,sp1-guest"
            ;;
        l2,risc0-guest)
            echo "-F l2,risc0-guest"
            ;;
        sp1-guest)
            echo "-F l1,sp1-guest"
            ;;
        risc0-guest)
            echo "-F l1,risc0-guest"
            ;;
        l2+sp1+sp1-guest)
            echo "--no-default-features -F l2,sp1,sp1-guest"
            ;;
        l2+risc0+sp1-guest)
            echo "--no-default-features -F l2,risc0,sp1-guest"
            ;;
        l1,sp1-guest,risc0-guest)
            echo "-F l1,sp1-guest,risc0-guest"
            ;;
        *)
            echo ""
            ;;
    esac
}

status_file="previous_status.txt"

declare -A previous_status
if [[ -f "$status_file" ]]; then
    while IFS=':' read -r pkg var stat; do
        if [[ -n "$pkg" && -n "$var" && -n "$stat" ]]; then
            key="$pkg:$var"
            previous_status["$key"]="$stat"
        fi
    done < "$status_file"
fi

declare -A current_status
failed_packages=()
failed_variants=()

for package in "${packages[@]}"; do
    pushd "$package" > /dev/null
    # Default compilation
    variant="default"
    extra=$(get_cargo_args "$package" "$variant")
    cmd="cd $package && cargo c -r $extra"
    echo "$cmd"
    if cargo c -r $extra; then
        current_status["$package:$variant"]="success"
    else
        current_status["$package:$variant"]="fail"
        failed_packages+=("$package")
        failed_variants+=("$variant")
    fi
    if [[ " ${special_packages[*]} " =~ " ${package} " ]]; then
        # SP1 compilation
        variant="sp1"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # RISC0 compilation
        variant="risc0"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
    fi
    if [[ " ${l2_packages[*]} " =~ " ${package} " ]]; then
        # l2
        variant="l2"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # l2+sp1
        variant="l2+sp1"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # l2+risc0
        variant="l2+risc0"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
    fi
    if [[ "$package" == "cmd/ethrex" ]]; then
        # l1,risc0-guest
        variant="l1,risc0-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # l1,sp1-guest
        variant="l1,sp1-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # l1,sp1-guest,risc0-guest
        variant="l1,sp1-guest,risc0-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
    fi
    if [[ "$package" == "crates/l2/prover/src/guest_program" ]]; then
        # l2,sp1-guest
        variant="l2,sp1-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # l2,risc0-guest
        variant="l2,risc0-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # sp1-guest
        variant="sp1-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # risc0-guest
        variant="risc0-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # l2+sp1+sp1-guest
        variant="l2+sp1+sp1-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
        # l2+risc0+sp1-guest
        variant="l2+risc0+sp1-guest"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "$cmd"
        if cargo c -r $extra; then
            current_status["$package:$variant"]="success"
        else
            current_status["$package:$variant"]="fail"
            failed_packages+=("$package")
            failed_variants+=("$variant")
        fi
    fi
    popd > /dev/null
done

# Compute diff
new_fails=()
new_successes=()
for key in "${!current_status[@]}"; do
    prev_status="${previous_status[$key]-unset}"
    curr_status="${current_status[$key]}"
    if [[ "$curr_status" == "fail" && ( "$prev_status" == "success" || "$prev_status" == "unset" ) ]]; then
        package="${key%:*}"
        variant="${key#*:}"
        new_fails+=("$package ($variant)")
    elif [[ "$curr_status" == "success" && "$prev_status" == "fail" ]]; then
        package="${key%:*}"
        variant="${key#*:}"
        new_successes+=("$package ($variant)")
    fi
done

echo "Diff from previous run:"
if [ ${#new_fails[@]} -eq 0 ] && [ ${#new_successes[@]} -eq 0 ]; then
    echo "No updates."
else
    if [ ${#new_fails[@]} -gt 0 ]; then
        echo "New failures:"
        for f in "${new_fails[@]}"; do
            echo "  $f"
        done
    fi
    if [ ${#new_successes[@]} -gt 0 ]; then
        echo "New successes:"
        for s in "${new_successes[@]}"; do
            echo "  $s"
        done
    fi
fi
echo ""

# Print current failed
if [ ${#failed_packages[@]} -eq 0 ]; then
    echo "All packages compiled successfully."
else
    echo "Current failed compilations:"
    for i in "${!failed_packages[@]}"; do
        package="${failed_packages[$i]}"
        variant="${failed_variants[$i]}"
        extra=$(get_cargo_args "$package" "$variant")
        cmd="cd $package && cargo c -r $extra"
        echo "Failed: $package ($variant): $cmd"
    done
fi
echo ""

# Save current status
> "$status_file"
for key in "${!current_status[@]}"; do
    echo "$key:${current_status[$key]}" >> "$status_file"
done

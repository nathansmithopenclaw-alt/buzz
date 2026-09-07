#!/usr/bin/env bash
# Sourced by rollout scripts. This is trusted, user-owned shell configuration;
# it contains public signing identifiers, never exported private key material.
if [[ -z "${BUZZ_MACOS_SIGNING_IDENTITY:-}" ]]; then
    BUZZ_SIGNING_CONFIG="${BUZZ_MACOS_SIGNING_CONFIG:-$HOME/.config/buzz/macos-signing.env}"
    if [[ -f "$BUZZ_SIGNING_CONFIG" ]]; then
        source "$BUZZ_SIGNING_CONFIG"
    fi
fi

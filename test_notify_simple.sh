#!/bin/bash
cargo run --example simple 2>&1 || echo "No example available"

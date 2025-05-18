# Monero FastSync

This crate is designed to be PoC for a fast wallet sync (decoding outputs and checking key images) implementation for [Saketo](https://saketo.io). Takes private spend key and block height from the user via CLI and starts scanning the chain, and displaying outputs that are spendable by the given key. It also checks for key images to see if they are already spent.

To use, clone the repo and run the project via CLI. You will see what's next.
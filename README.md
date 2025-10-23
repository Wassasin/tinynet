# Tinynet
Toolkit for tiny networks for Rust Embedded devices.

Less powerful but more compact than ethernet, it is suited for RS485 busses and long range low bandwidth RF.

Provides distinct protocols for specific usecases.
* `ptp_packets` can be used on byte-based half-duplex connections like some RS485 busses.
* `routing` provides general tooling for routing packets between busses.
* `protocols` implement support for higher level protocols like address discovery.

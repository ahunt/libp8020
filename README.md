# libp8020

A collection of utils (and perhaps a library in future) for 8020A's. This may or
may not work with your 8020A, any 8020, 8020M, or other devices.

To run a fit test (IMPORTANT: this is experimental, calculations have not been
validated), invoke:

    cargo run --bin test

To simply observe the portacount's serial output, invoke:

    cargo run --bin spy

## Resources

* PORTA COUNT ® Plus Model 8020 Technical Addendum:
  https://tsi.com/getmedia/0d5db6cd-c54d-4644-8c31-40cc8c9d8a9f/PortaCount_Model_8020_Technical_Addendum_US?ext=.pdf
  Explains the protocol (mostly, modulo mistakes - see errata below).

## Technical Addendum Errata

(Based on working with my 8020A, YMMV.)

* Switch valve off response (Technical Addendum P. 14): my 8020A responds with
  "VF" (which matches the command to switch valve off) instead of "VO". (By
  comparison, both the switch valve on command and response - in the addendum
  and in reality - are "VN".)


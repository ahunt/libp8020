# libp8020

A little library that allows you to run fit tests on an 8020(A). This may or may
not work with your 8020A, any 8020, 8020M, or other devices.

See https://github.com/ahunt/incolata for the only known usage.

## Build

```
cargo build

# To run tests:
cargo test
```

## Fuzzing

```
# To list targets
cargo fuzz list

# To fuzz a target
cargo fuzz run <TARGET> --jobs=N
```

## Resources

* PORTA COUNT ® Plus Model 8020 Technical Addendum:
  https://tsi.com/getmedia/0d5db6cd-c54d-4644-8c31-40cc8c9d8a9f/PortaCount_Model_8020_Technical_Addendum_US?ext=.pdf
  Explains the protocol (mostly, modulo mistakes - see errata below).

## Technical Addendum Errata

(Based on working with my 8020A, YMMV.)

* Switch valve off response (Technical Addendum p. 14): my 8020A responds with
  "VF" (which matches the command to switch valve off) instead of "VO". (By
  comparison, both the switch valve on command and response - in the addendum
  and in reality - are "VN".)
* Beep command: the maximum duration supported by my 8020A is 60 deciseconds
  (not 99 as claimed in the addendum). E.g. `B61` -> `EB61` (and no beep).
   * libp8020 will print an error for durations above 60 deciseconds.
* Serial numbers can be > 5 chars (Technical Addendum p. 16 shows "SS   vvvvv"
  with an emphasis on three spaces, but specifying niether the length of the
  serial number, nor permitted chars).

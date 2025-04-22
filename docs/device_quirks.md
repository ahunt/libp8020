# Device Quirks

This doc documents differences seen while testing 

## Serial numbers

(Based on a limited sample size, and might be incomplete.)n

* 8020A: 8024XXXX
* 8020Mgen1: `26XXX`.

## 8020Mgen1 behaviour

The 8020Mgen1 appears to be slower than the 8020A, meaning larger waits are
needed between commands:
https://github.com/ahunt/incolata/issues/10

The settings list also contains additional entries, their meaning is currently
unknown (to me).

Other than that, the same protocol does work with the 8020Mgen1, including
beeps. Only the external control indicator is missing (the LED is there on the
PCB, but not visible from the outside).

## 8020Mgen2 serial connection

The 8020Mgen2 does have a serial port, but:

* It's labelled differently.
* The manual states that it shouldn't be used (at least not for PC connections).

I haven't tested this port, it might be a serial port with standard ordering (or
ordering different than on the 8020(A)).


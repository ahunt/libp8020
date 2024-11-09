# Test Design

## Goal

Devise a way to represent test protocols. This representation should support all
comon existing CNC protocols (OSHA full/OSHA modified/ISO); supporting new forms
of protocols is a nice-to-have.

## Background

Three broad classes of test types (are known to) exist:

- Strictly periodic tests: the test consists of series of identical exercises,
  where each exercise contains the same number of ambient purges + ambient
  samples + specimen purges + specimen samples. The final exercise is followed
  by another set of ambient purges + ambient samples. The 8020 defaults to this
  mode of testing.
- Periodic tests, with varying specimen sample periods. Similar to above, except
  that each exercise might have a different specimen sample time. FitPro
  defaults to this mode of testing - FitPro's OSHA protocol uses a
  shorter specimen sample period duration of 15s for exercise 6 (grimace). The
  8020 supports this mode of testing too, but I suspect that few people use it:
  changing specimen sample times is only possible via external software (it
  cannot be configured directly on the device) - the relevant commands are
  documented in the Technical Addendum.
- Fast protocol tests (or "Modified ... protocol" in OSHA parlance), only
  available in software (or, more accurately, not possible on an 8020 without
  software):

  1. Ambient Purges + Ambient Samples + Specimen Purges (in OSHA parlance: all
     of this seems to be considered the "ambient sample" of 20s, which is
     actually divided into 4+5+11s).
  2. A series of exercises, each consisting of only specimen samples (each
     sample period typically has the same duration, but FitPro allows it to be
     configured per-exercise).
  3. Ambient Purges + Ambient Samples.

### Specimen purges for fast protocols

The fast protocols do not include a specimen purge beteween each exercise (they
only purge immediately after the ambient samples). However (due to the length of
the twin tubes) there is a delay in detecting/measuring a change in particle
concentrations within the specimen (perhaps 4-5s, but I haven't measured with
great precision). This means that the first 4-5 samples within a given specimen
sample period actually reflect the previous exercise.

This is unlikely to matter much for the overall average fit-factor calculation,
but may make it harder to see the true impact of each exercise individually -
unless, that is, the subject is instructed to start the next exercise 4-5s
before the test implementation switches exercises. The other obvious alternative
might be to always include a specimen purge period (to be discussed below) -
TSI's implementation of the fast OSHA protocols doesn't do this obviously, but
perhaps other protocol designers might make use of such a possibility?

## Design

### The simple version

If we merely wish to support the protocols above, the following test
configuration representation would be sufficient:

```rust
struct TestConfig {
  ambient_purges: u8,
  ambient_samples: u8,
  specimen_purges: u8,
  // The number of samples for each exercise. The number of exercises is defined
  // by the number of entries in this list.
  specimen_samples: Vec<u8>,
  // Ambient samples only at beginning and end (aka fast protocol).
  no_intermediate_ambient_samples: bool,
}
```

But is this the best we can do?

Moreover, this (probably) leads to a somewhat messy implementation with
special-casing depending on whether no_intermediate_ambient_samples is set.

```rust
let mut first = true
for specimen_samples in test_config.specimen_samples.iter() {
  if first || !test_config.no_intermediate_ambient_samples {
    first = false;
    // Do the ambient thing, including specimen purge
  }
  // Do the specimen sample thing
}
// Do another ambient thing.
```

### A more general design - assembling a test from arbitrary stages

We can allow users to declare an arbitrary sequece of ambient and specimen
periods, i.e. allow users to provide a list containing multiple sections, or
"stages":

* Ambient stages: declares ambient purges + ambient samples, and *maybe*
  specimen purges (see more discussion below).
* Specimen stage: *maybe* declares specimen purges (more below), and specimen
  samples. This could also include the exercise name... in fact this should
  probably be called exercise stage not a specimen stage?

The fast protocol could thus be defined as Ambient(4,5),Specimen(11,40),
Specimen(0,40),Specimen(0,40),Specimen(0,40), Ambient(4,5).

Note: the ambient purges+ambient samples+specimen purges could be declared
once, for the entire test (instead of per stage) - even for the fast protocol.
Such an approach is more concise, but it reduces flexibility. I don't expect
users to be editing test configurations frequently, therefore specifying these
times per-ambient-period and not per-test is probably acceptable (from the
perspective of effort required, & duplication involved when creating a
protocol), and is nice since some users may wish to experiment with new
protocols.

**Should specimen purges be declared on the ambient or specimen period?**

This question only matters for tests that are not periodic. For periodic tests,
there must be a specimen purge period for every exercise, therefore it doesn't
really matter where specimen purges are declared (there's going to be N sets of
them for N exercises)

For the fast protocols: declaring specimen purges on the ambient stage is
more efficient, because specimen purges occur only after ambient sampling :
otherwise we'd have to declare a redundant specimen purge count of 0s for
exercises 2..=4.

Given the caveats explained in the "Specimen purges for fast protocols" section
above, users who design their own protocols may wish to configure specimen
purges between every exercise. (An  alternative would be to allow users to
an exercise change "delay" in the UI - the UI could claim that the next exercise
has started 4-5s in advance of the test implementation switching to the next
exercise. Or, alternatively, the test implementation would consider samples
during the 4-5s after an exercise change to belong to the previous exercise. But
this option is more complex, confusing to reason about, and imprecise if we
cannot accurately determine the delay - and I suspect that the delay varies by
machine, tubing length, and/or depending on whether an N95 companion is in use.)

Therefore, specimen purges will be part of the specimen stage, as this gives our
users more flexibility. To run an OSHA fast protocol, the specimen purge count
will therefore be 0s for exercises 2..=4. But one can easily imagine something
like a RapidCrash protocol consisting of ambient stage + 3 exercises + ambient
stage, and so on..., with a 4-5s purge between each exercise. (The purge is
expected to matter less for Crash2.5 style protocols, but it might matter more
for an imaginary protocol with a talking exercise followed by breathing exercise
without any intermediate ambient stage.)

### Config representation

Users will want to specify their own configurations. (In fact, most of the
design above is pointless if we do not allow users to specify their own
configs.)

Options:

* UI: users enter their config in a UI. But... a UI is expensive, and we still
  need to store the config on disk somehow.
* CSV (file, or copy pasted into a textbox).
* JSON.
* JSON-serialised-thrift (TJSONProtocol), TextProto, etc.
* Binary-serialised-thrift (TBinaryProtocol), BinaryProto, etc. (Horrible for
  users.)
* Custom binary representation. (Ughh. Although it would be the most space
  efficient...)
* Prose, parsed by AI. "An ambient purge of 4s followed by 5s of ambient
  samples" etc. Sadly, the author hasn't the slightest notion of how AI works.

CSV is more than sufficient, avoids extraneous dependencies, and is probably
easiest for users to edit. (Changing this decision in future is likely to be
cheap anyway - this decision is expected to have no expensive long-term
consequences.) A UI to edit it could be added later, but tbh that seems
unnecessary (8020 owners seem to be quite a technical bunch).

```
# This line will be ignored
TEST,"Your protocol name","protocolShortName"
AMBIENT,4,5
EXERCISE,11,40,"Hop on one leg, whilst reciting this document"
AMBIENT,4,5
```

### Validation

The above representation allows users to supply nonsensical configurations.
Enforcing the following constraints seems sensible:

 * Each test begins with an AMBIENT stage.
 * Each test ends with an AMBIENT stage.
 * Each test contains at least one EXERCISE stage.
 * An AMBIENT period may not be followed by another AMBIENT stage.

NOTE: the first two requirements aren't strictly necessary. These requirements
ensure that there is always an ambient period before and after each exercise.
But it's hard to conceive of situations where users would not want to do this
and/or where it would be useful to waive this requirement. And making this
assumption simplifies implementation (see test algorithm section below).
 
The following additional validation could be considered - these rules should
only trigger a warning but not result in rejection of a test configuration:

* The interval between each AMBIENT period is no more than e.g. 5 minutes.

### Test algorithm

The fit factor for a given exercise is the average ambient particle
concentration for the closest ambient stages before and after the exercise,
divided by the average specimen particle concentration.

An exercise's fit factor cannot be definitively calculated until the succeeding
ambient stage has completed. However, users probably want to be able to see an
interim fit factor sooner. Therefore we can calculate an interim fit-factor
using only the prior ambient period's average particle concentration. This
approach can be extended to allow us to continuously calculate an interim fit
factor throughout each exercise (using the average specimen particle
concentration for all datapoints received so far). (This is different from the
"Live" fit-factor which is the fit-factor at any given point in time, calculated
as previous_ambient_concentration_avg/momentary_specimen_concentration.)

* For each ambient purge datapoint: do nothing.
* For each amient sample datapoint: store.
* For each specimen purge datapoint: do nothing.
* For each specimen sample datapoint: store, calculate live FF (this datapoint),
  and interim FF (all specimen datapoints for this exercise).
* After each ambient stage: calculate actual FF for each exercise since the
  previous ambient stage.

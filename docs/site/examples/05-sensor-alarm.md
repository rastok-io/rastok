---
title: 'Sensor alarm: make time an input'
description: Isolate hysteresis state by sensor, detect a missed heartbeat with a bounded join, and advance through a virtual day without waiting a day.
teaches: Private state, instance keys, hysteresis, timed joins, JoinExpired, explicit logical time
graph: examples/sensor-alarm.rkg
golden: examples/traces/sensor-alarm/sensors.virtual_day.rkc
command: rastok simulate examples/sensor-alarm.rkg --compare examples/traces/sensor-alarm
expected: Three scenarios pass; pump-a trips and clears, pump-b stays isolated, and one unanswered probe expires.
break_it: 'Change data.value > 120 to data.value > 130, then rerun against its reference traces.'
limits:
  - The Lab authors probes and heartbeats; there is no live sensor or periodic Driver here.
  - JoinExpired means an unmatched rendezvous; an unsolicited unmatched heartbeat can expire too.
  - The reported duration is a local debug-build observation, not a throughput guarantee.
order: 5
---

This program tracks each sensor's readings, emits alarm changes and reports a missing heartbeat.

The partner example tested a response and a timeout. Now ask a harder
question: how do you test silence? Keep state per sensor, distinguish a
threshold crossing from noise, and make the waiting period an explicit input
to your scenario.

The [file formats](../guide/09-command-line.md#file-formats) are `.rkg` for
the program and Lab, `.rkc` for a reviewed scenario reference and `.rkr`
for a real execution recording.

## Run

From the repository root, with `rastok` installed:

```sh
rastok simulate examples/sensor-alarm.rkg --compare examples/traces/sensor-alarm
```

```text
PASS sensors.hysteresis
PASS sensors.silent_sensor
PASS sensors.virtual_day
3 scenario(s), 0 failed
```

## Read the program

First follow the readings into the keyed stateful Operator: its two thresholds
prevent an alarm from oscillating around one limit. Then find the `watchdog`
join. A probe and heartbeat rendezvous within a bounded interval; a missing
partner produces `JoinExpired`, whose route is as explicit as a normal output.
Read `advance` in the Lab last: time comes from the scenario, not from sleeps
hidden in the application.

There is no Connector in this program. The [Lab reference](../reference/lab.md)
explains expectations and observation windows; `advance` supplies virtual time.

## Expected result

The golden's `sensors.virtual_day` follows one unanswered probe, not the
separate hysteresis scenario; the advance visits its five-minute deadline.

```mermaid
sequenceDiagram
    participant lab as Lab: sensors.virtual_day
    participant app as sensor_alarm
    participant watchdog as watchdog
    lab->>app: probe = pump-a
    app->>watchdog: probe = pump-a
    Note over watchdog: Wait for heartbeat within 5m
    lab->>lab: advance 24h
    Note over watchdog: At 5m: no matching heartbeat
    watchdog-->>app: expired to silent = JoinExpired
    Note over lab,app: Advance completes at 24h, no alarm emitted
```

Pump-a trips above 120, stays tripped at 100, and clears below 80. Pump-b has
independent state. A keyed join pairs a probe with a heartbeat within five
minutes; only the unanswered probe expires in the silent-sensor scenario.
The final scenario advances twenty-four virtual hours in one authored action.

With Ruby and the built `target/debug/rastok` available, this command measures
process wall time on your machine; it does not change the scenario's logical time:

```sh
ruby -e '3.times { t = Process.clock_gettime(Process::CLOCK_MONOTONIC); ok = system(*ARGV); puts format("elapsed_cli_seconds=%.6f", Process.clock_gettime(Process::CLOCK_MONOTONIC) - t); exit 1 unless ok }' target/debug/rastok simulate examples/sensor-alarm.rkg
```

## Break it

Raise the trip threshold from 120 to 130. The sample at 121 no longer trips,
and the original assertion and golden fail. Alternatively, remove the route
from `watchdog.expired`: the checker refuses `E-UNWIRED-OUTPUT` before execution.

```sh
rastok simulate examples/sensor-alarm.rkg --compare examples/traces/sensor-alarm
```

```text
FAIL sensors.hysteresis
     expect app.alarm where data.sensor == "pump-a" && data.state == "tripped"  (sensor-alarm.rkg:54)
     nothing observed
     where data.sensor == "pump-a" && data.state == "tripped" is false for 0 of 0 deliveries; count 0, expected 1
     expect app.alarm where data.state == "normal"  (sensor-alarm.rkg:60)
     nothing observed
     where data.state == "normal" is false for 0 of 0 deliveries; count 0, expected 1
     golden divergence: record 3: emissions(23).len; expected 1; got 0; cause: TRANSACTION "hysteresis" · trigger "p:in" · cause_id absent
PASS sensors.silent_sensor
PASS sensors.virtual_day
3 scenario(s), 1 failed
```

Restore the checked-in source (this discards your edits to this example):

```sh
git checkout -- examples/sensor-alarm.rkg
```

## Read the golden difference

The threshold edit changes a recorded Operator transaction, including its
state delta and emitted alarm. The diff gives the first changed record,
expected/actual values and causal trigger. The missing expiry route is a
static error, not a runtime golden difference.

## Carry forward

[The Lab](../guide/04-the-lab.md) explains this example's virtual clock and
the observation window used by its expectations.

Time, like an external response, is something you can supply deliberately.
You can retain a missed heartbeat as a small regression scenario instead of
waiting for the fault to recur on equipment. Types, state ownership and the
same causal comparison still apply; the device domain needs no special Lab.

The core evaluating this graph has no clock, network access or randomness;
time and external outcomes enter as explicit inputs. CI compiles `rastok-core`
for targets without an operating system: `thumbv8m.main-none-eabihf`,
`thumbv7m-none-eabi` and `riscv32imac-unknown-none-elf`. It also runs the
RP2350 spike under QEMU's Cortex-M33 emulation. Running this sensor example
on a physical board is not part of the alpha.

Previous: [Webhook to partner](04-webhook-to-partner.md).
Next: [Agent gateway](06-agent-gateway.md): keep a separate resident process
for each conversation and interleave several turns. Before moving on, try
writing a second silent-sensor scenario and explaining its first expiry from
the journal.

## Limits

This is a requested probe/heartbeat rendezvous, not automatic periodic polling.
No sensor protocol, process or paid model runs. An unmatched value on either
join input can produce `JoinExpired`; the scenarios explicitly control which
side is silent. Expiry DATA has no sensor field: its keyed instance and causal
history supply that context. The benchmark tasks informed the example, but
no claim is made that a published model wave authored it.

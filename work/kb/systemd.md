# systemd native service deadlines

- [V 2026-11-01] In v262, `RuntimeMaxSec` expiry enters stop; without an
  ExecStop command the manager enters STOP_SIGTERM. `KillSignal` determines the
  actual initial signal independently of this state name. `service_enter_signal`
  arms TimeoutStopSec when killed processes remain. Source:
  https://github.com/systemd/systemd/blob/v262/src/core/service.c
- [V 2026-11-01] With the default terminate failure mode and SendSIGKILL=yes,
  STOP_SIGTERM, STOP_SIGKILL, STOP_POST, FINAL_SIGTERM and FINAL_SIGKILL can
  each consume a separate TimeoutStopSec. A SIGKILL setting does not eliminate
  those state names or make manager/kernel operations universally bounded.
  Source: `service_dispatch_timer` and `service_enter_signal`,
  https://github.com/systemd/systemd/blob/v262/src/core/service.c

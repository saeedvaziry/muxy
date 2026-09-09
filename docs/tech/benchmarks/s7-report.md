
### avg_read_kb (KB)

| candidate | pty |
| --- | ---: |
| portable-pty | 0.020 |
| portable-pty+batch | 0.998 |
| pty-process | 0.020 |
| rustix-openpty | 0.020 |

### echo_rtt_p50_us (us)

| candidate | pty |
| --- | ---: |
| portable-pty | 0.958 |
| pty-process | 0.959 |
| rustix-openpty | 0.958 |

### echo_rtt_p99_us (us)

| candidate | pty |
| --- | ---: |
| portable-pty | 1.375 |
| pty-process | 2.583 |
| rustix-openpty | 4.833 |

### read_mb_s (MB/s)

| candidate | pty |
| --- | ---: |
| portable-pty | 17.0 |
| portable-pty+batch | 1.399 |
| pty-process | 16.9 |
| rustix-openpty | 16.7 |

### reader_cpu_s (s)

| candidate | pty |
| --- | ---: |
| portable-pty | 9.344 |
| portable-pty+batch | 5.105 |
| pty-process | 9.417 |
| rustix-openpty | 9.521 |

### resize_us (us)

| candidate | pty |
| --- | ---: |
| portable-pty | 16.9 |
| pty-process | 10.2 |
| rustix-openpty | 11.7 |

### spawn_exit_ms (ms)

| candidate | pty |
| --- | ---: |
| portable-pty | 4.624 |
| pty-process | 3.018 |
| rustix-openpty | 3.267 |


### client_cpu_s (s)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 0.583 | 0.482 | |
| stdio/prio | 0.012 | 0.488 | |
| tcp/naive | 0.619 | 0.897 | |
| tcp/prio | 0.011 | 0.895 | |
| unix/naive | 0.619 | 0.510 | 1.601 |
| unix/prio | 0.012 | 0.466 | 0.129 |

### frames_delivered (n)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 7050 | 22200 | |
| stdio/prio | 187.0 | 22199 | |
| tcp/naive | 7050 | 22200 | |
| tcp/prio | 142.0 | 22174 | |
| unix/naive | 7050 | 22200 | 22200 |
| unix/prio | 151.0 | 22197 | 2273 |

### frames_merged (n)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 0.000 | 0.000 | |
| stdio/prio | 6865 | 31.0 | |
| tcp/naive | 0.000 | 0.000 | |
| tcp/prio | 6916 | 30.0 | |
| unix/naive | 0.000 | 0.000 | 0.000 |
| unix/prio | 6901 | 129.0 | 19934 |

### frames_produced (n)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 7050 | 22200 | |
| stdio/prio | 7050 | 22200 | |
| tcp/naive | 7050 | 22200 | |
| tcp/prio | 7050 | 22200 | |
| unix/naive | 7050 | 22200 | 22200 |
| unix/prio | 7050 | 22200 | 22200 |

### rtt_max_ms (ms)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 227.2 | 100.0 | |
| stdio/prio | 224.4 | 96.2 | |
| tcp/naive | 253.8 | 106.1 | |
| tcp/prio | 256.7 | 107.4 | |
| unix/naive | 255.0 | 102.9 | 371.8 |
| unix/prio | 255.9 | 107.2 | 285.8 |

### rtt_p50_ms (ms)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 0.642 | 0.181 | |
| stdio/prio | 69.5 | 0.270 | |
| tcp/naive | 8.008 | 0.445 | |
| tcp/prio | 54.5 | 0.570 | |
| unix/naive | 0.465 | 0.214 | 51.5 |
| unix/prio | 63.9 | 0.236 | 136.2 |

### rtt_p99_ms (ms)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 96.2 | 1.460 | |
| stdio/prio | 198.5 | 2.730 | |
| tcp/naive | 98.0 | 5.580 | |
| tcp/prio | 135.8 | 5.235 | |
| unix/naive | 112.5 | 1.966 | 282.2 |
| unix/prio | 167.1 | 4.147 | 263.3 |

### server_cpu_s (s)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 0.381 | 1.268 | |
| stdio/prio | 0.442 | 1.768 | |
| tcp/naive | 0.390 | 1.654 | |
| tcp/prio | 0.445 | 2.080 | |
| unix/naive | 0.414 | 1.273 | 1.725 |
| unix/prio | 0.447 | 1.565 | 1.329 |

### server_rss_peak_delta (MB)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 114.6 | 2.609 | |
| stdio/prio | 19.0 | 2.906 | |
| tcp/naive | 138.9 | 2.641 | |
| tcp/prio | 19.0 | 2.891 | |
| unix/naive | 140.4 | 2.719 | 128.2 |
| unix/prio | 19.2 | 2.891 | 3.266 |

### throughput (MB/s)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 149.6 | 8.335 | |
| stdio/prio | 5.191 | 8.334 | |
| tcp/naive | 143.7 | 8.327 | |
| tcp/prio | 4.090 | 8.312 | |
| unix/naive | 136.2 | 8.331 | 0.951 |
| unix/prio | 4.303 | 8.320 | 0.452 |

### wall_s (s)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 1.115 | 9.706 | |
| stdio/prio | 0.641 | 9.700 | |
| tcp/naive | 1.177 | 9.711 | |
| tcp/prio | 0.677 | 9.716 | |
| unix/naive | 1.217 | 9.709 | 93.6 |
| unix/prio | 0.679 | 9.715 | 10.0 |

### wire_mb (MB)

| candidate | asap-fire-x100 | paced-monitor-x100 | slow-client-x100 |
| --- | ---: | ---: | ---: |
| stdio/naive | 165.6 | 80.8 | |
| stdio/prio | 3.311 | 80.8 | |
| tcp/naive | 165.6 | 80.8 | |
| tcp/prio | 2.726 | 80.7 | |
| unix/naive | 165.6 | 80.8 | 80.9 |
| unix/prio | 2.903 | 80.8 | 4.543 |

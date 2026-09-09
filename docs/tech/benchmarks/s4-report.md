
### bytes_per_frame (B)

| candidate | w2-plain | w3-buildlog | w4-vim | w5-monitor | w6-fire | w7-unicode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| cells/bincode/none | 3472 | 17817 | 12051 | 33419 | 68706 | 23877 |
| cells/bincode/zstd1 | 212.3 | 1249 | 1114 | 2094 | 5733 | 2172 |
| cells/msgpack/none | 4274 | 22122 | 15057 | 41081 | 94987 | 29238 |
| cells/msgpack/zstd1 | 239.0 | 1282 | 1139 | 1989 | 12547 | 2291 |
| cells/postcard/lz4 | 462.6 | 2302 | 2170 | 7123 | 27313 | 3680 |
| cells/postcard/none | 3098 | 16245 | 11050 | 29908 | 75284 | 22246 |
| cells/postcard/zstd1 | 204.8 | 1220 | 1013 | 2075 | 11098 | 2210 |
| cells/postcard/zstd3 | 230.6 | 1205 | 1007 | 2004 | 10891 | 2064 |
| cells/postcard/zstddict | 251.0 | 1560 | 1039 | 2098 | 11099 | 2537 |
| cells/postcard/zstdstream | 163.5 | 816.6 | 618.5 | 1863 | 10891 | 1607 |
| cells/prost/none | 3945 | 21629 | 13850 | 37563 | 94980 | 29326 |
| cells/prost/zstd1 | 239.4 | 1649 | 1119 | 2339 | 12862 | 2347 |
| runs/bincode/none | 880.9 | 3042 | 1751 | 4939 | 63793 | 5483 |
| runs/bincode/zstd1 | 213.4 | 818.3 | 589.4 | 1583 | 8308 | 901.0 |
| runs/msgpack/none | 984.9 | 3239 | 1863 | 5188 | 78911 | 5718 |
| runs/msgpack/zstd1 | 246.0 | 860.4 | 622.2 | 1613 | 12771 | 928.0 |
| runs/postcard/lz4 | 283.3 | 1076 | 856.7 | 2705 | 25226 | 1034 |
| runs/postcard/none | 780.9 | 2857 | 1703 | 4694 | 60847 | 5246 |
| runs/postcard/zstd1 | 203.5 | 784.1 | 578.3 | 1557 | 11631 | 878.0 |
| runs/postcard/zstd3 | 202.7 | 775.6 | 559.4 | 1590 | 10638 | 860.5 |
| runs/postcard/zstddict | 182.2 | 276.3 | 665.1 | 1667 | 11540 | 924.5 |
| runs/postcard/zstdstream | 197.0 | 490.4 | 274.9 | 1415 | 11386 | 587.5 |
| runs/prost/none | 1079 | 3544 | 1932 | 5377 | 96867 | 6062 |
| runs/prost/zstd1 | 285.0 | 955.3 | 676.2 | 1696 | 14104 | 1057 |
| vt/raw/lz4 | 443.5 | 1290 | 901.6 | 2711 | 27827 | 983.0 |
| vt/raw/none | 1280 | 3767 | 2099 | 5379 | 124603 | 5330 |
| vt/raw/zstd1 | 152.9 | 829.1 | 552.6 | 1513 | 14643 | 801.0 |
| vt/raw/zstd3 | 165.8 | 812.3 | 538.0 | 1564 | 13483 | 787.0 |
| vt/raw/zstddict | 242.6 | 541.7 | 661.7 | 1684 | 14731 | 844.0 |
| vt/raw/zstdstream | 262.6 | 561.2 | 275.3 | 1365 | 14454 | 550.0 |

### decode_us_per_frame (us)

| candidate | w2-plain | w3-buildlog | w4-vim | w5-monitor | w6-fire | w7-unicode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| cells/bincode/none | 9.877 | 32.5 | 18.8 | 49.7 | 125.6 | 53.8 |
| cells/bincode/zstd1 | 9.310 | 40.8 | 34.4 | 67.0 | 180.5 | 64.9 |
| cells/msgpack/none | 10.7 | 49.9 | 31.8 | 83.3 | 251.4 | 74.1 |
| cells/msgpack/zstd1 | 11.9 | 58.7 | 51.1 | 106.3 | 343.1 | 80.1 |
| cells/postcard/lz4 | 5.968 | 26.9 | 17.9 | 46.5 | 147.5 | 40.4 |
| cells/postcard/none | 6.842 | 24.4 | 14.8 | 41.1 | 122.3 | 39.5 |
| cells/postcard/zstd1 | 8.162 | 33.4 | 24.8 | 55.6 | 218.1 | 47.3 |
| cells/postcard/zstd3 | 8.297 | 32.6 | 27.1 | 55.4 | 232.2 | 45.8 |
| cells/postcard/zstddict | 12.5 | 46.5 | 35.2 | 62.6 | 265.1 | 56.5 |
| cells/postcard/zstdstream | 7.831 | 32.7 | 22.4 | 63.9 | 230.0 | 45.9 |
| cells/prost/none | 10.1 | 53.9 | 29.3 | 82.6 | 213.8 | 74.9 |
| cells/prost/zstd1 | 13.1 | 64.8 | 40.4 | 104.5 | 304.2 | 83.6 |
| runs/bincode/none | 3.164 | 6.870 | 2.556 | 6.480 | 227.2 | 11.4 |
| runs/bincode/zstd1 | 5.286 | 11.4 | 6.150 | 13.5 | 309.0 | 14.5 |
| runs/msgpack/none | 4.502 | 10.3 | 3.382 | 8.486 | 345.6 | 15.7 |
| runs/msgpack/zstd1 | 6.924 | 12.6 | 6.842 | 14.9 | 450.6 | 16.8 |
| runs/postcard/lz4 | 3.527 | 7.597 | 3.036 | 7.172 | 265.5 | 11.3 |
| runs/postcard/none | 3.043 | 9.713 | 2.614 | 7.593 | 275.5 | 13.3 |
| runs/postcard/zstd1 | 5.148 | 10.3 | 5.541 | 11.8 | 378.5 | 13.7 |
| runs/postcard/zstd3 | 4.800 | 9.616 | 5.633 | 12.7 | 322.0 | 12.7 |
| runs/postcard/zstddict | 8.434 | 15.6 | 9.175 | 16.6 | 370.9 | 22.2 |
| runs/postcard/zstdstream | 4.384 | 10.7 | 4.388 | 11.5 | 333.0 | 37.1 |
| runs/prost/none | 4.257 | 10.4 | 2.969 | 9.049 | 409.0 | 16.6 |
| runs/prost/zstd1 | 5.448 | 12.4 | 6.055 | 14.9 | 501.3 | 18.2 |
| vt/raw/lz4 | 0.333 | 1.667 | 0.768 | 1.480 | 26.1 | 4.479 |
| vt/raw/none | 0.014 | 0.065 | 0.022 | 0.017 | 0.033 | 0.104 |
| vt/raw/zstd1 | 3.465 | 5.296 | 3.231 | 5.966 | 88.9 | 7.438 |
| vt/raw/zstd3 | 2.417 | 4.306 | 3.150 | 6.897 | 72.5 | 5.479 |
| vt/raw/zstddict | 7.141 | 9.894 | 7.387 | 10.6 | 94.4 | 10.6 |
| vt/raw/zstdstream | 1.972 | 4.796 | 1.600 | 5.344 | 98.7 | 6.583 |

### encode_us_per_frame (us)

| candidate | w2-plain | w3-buildlog | w4-vim | w5-monitor | w6-fire | w7-unicode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| cells/bincode/none | 4.278 | 22.1 | 16.3 | 40.5 | 75.1 | 27.2 |
| cells/bincode/zstd1 | 19.9 | 45.4 | 39.1 | 64.3 | 180.7 | 55.9 |
| cells/msgpack/none | 8.720 | 38.1 | 27.4 | 66.8 | 174.7 | 59.0 |
| cells/msgpack/zstd1 | 16.0 | 58.7 | 46.8 | 104.3 | 309.9 | 85.3 |
| cells/postcard/lz4 | 6.342 | 34.5 | 26.2 | 66.1 | 184.6 | 52.8 |
| cells/postcard/none | 5.302 | 25.0 | 18.2 | 47.6 | 115.8 | 38.9 |
| cells/postcard/zstd1 | 10.6 | 45.8 | 36.1 | 75.4 | 253.5 | 62.2 |
| cells/postcard/zstd3 | 10.5 | 46.9 | 35.4 | 73.2 | 221.5 | 64.5 |
| cells/postcard/zstddict | 22.6 | 71.2 | 50.3 | 90.5 | 290.2 | 94.4 |
| cells/postcard/zstdstream | 9.124 | 41.2 | 33.4 | 79.3 | 260.6 | 59.7 |
| cells/prost/none | 11.5 | 55.5 | 29.9 | 78.0 | 158.2 | 62.2 |
| cells/prost/zstd1 | 15.9 | 76.6 | 52.2 | 114.8 | 297.4 | 89.9 |
| runs/bincode/none | 11.4 | 18.6 | 12.6 | 30.1 | 256.0 | 24.2 |
| runs/bincode/zstd1 | 14.3 | 27.5 | 22.0 | 42.8 | 382.3 | 32.6 |
| runs/msgpack/none | 13.0 | 27.7 | 10.9 | 34.0 | 323.7 | 33.9 |
| runs/msgpack/zstd1 | 13.6 | 31.5 | 19.5 | 47.8 | 459.6 | 36.5 |
| runs/postcard/lz4 | 9.690 | 22.8 | 12.3 | 37.5 | 363.5 | 28.8 |
| runs/postcard/none | 10.3 | 22.8 | 10.6 | 32.8 | 292.3 | 30.3 |
| runs/postcard/zstd1 | 17.7 | 27.5 | 18.7 | 43.3 | 438.6 | 34.1 |
| runs/postcard/zstd3 | 14.2 | 25.1 | 21.8 | 54.4 | 1723 | 32.9 |
| runs/postcard/zstddict | 27.4 | 47.3 | 31.7 | 61.2 | 478.8 | 63.8 |
| runs/postcard/zstdstream | 11.2 | 28.0 | 14.6 | 44.6 | 434.0 | 34.2 |
| runs/prost/none | 11.5 | 22.5 | 12.0 | 39.0 | 481.0 | 33.4 |
| runs/prost/zstd1 | 14.0 | 32.7 | 18.9 | 48.3 | 628.9 | 40.2 |
| vt/raw/lz4 | 14.4 | 27.6 | 12.5 | 30.3 | 635.8 | 27.0 |
| vt/raw/none | 25.6 | 29.1 | 10.7 | 39.4 | 584.3 | 22.2 |
| vt/raw/zstd1 | 16.6 | 24.6 | 15.4 | 37.3 | 683.0 | 35.4 |
| vt/raw/zstd3 | 14.4 | 22.5 | 20.0 | 44.0 | 686.8 | 31.3 |
| vt/raw/zstddict | 27.6 | 40.2 | 28.7 | 57.2 | 720.2 | 56.0 |
| vt/raw/zstdstream | 15.3 | 20.8 | 13.2 | 38.4 | 714.4 | 26.4 |

### frames (n)

| candidate | w2-plain | w3-buildlog | w4-vim | w5-monitor | w6-fire | w7-unicode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| stream | 67.0 | 9.000 | 109.0 | 600.0 | 19.0 | 2.000 |

### raw_kb (KB)

| candidate | w2-plain | w3-buildlog | w4-vim | w5-monitor | w6-fire | w7-unicode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| stream | 204800 | 26923 | 163.7 | 2735 | 57523 | 4729 |

### snapshot_kb (KB)

| candidate | w2-plain | w3-buildlog | w4-vim | w5-monitor | w6-fire | w7-unicode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| cells/bincode/none | 18.2 | 87.3 | 0.540 | 275.2 | 341.1 | 121.5 |
| cells/bincode/zstd1 | 0.771 | 3.585 | 0.106 | 6.389 | 27.7 | 4.869 |
| cells/msgpack/none | 22.4 | 108.4 | 0.690 | 337.4 | 471.5 | 148.8 |
| cells/msgpack/zstd1 | 0.826 | 3.229 | 0.113 | 5.881 | 62.1 | 7.190 |
| cells/postcard/lz4 | 2.099 | 6.775 | 0.215 | 21.3 | 135.2 | 8.881 |
| cells/postcard/none | 16.2 | 79.6 | 0.491 | 245.4 | 373.7 | 113.0 |
| cells/postcard/zstd1 | 0.790 | 3.409 | 0.105 | 6.002 | 54.9 | 5.194 |
| cells/postcard/zstd3 | 0.860 | 3.410 | 0.105 | 5.901 | 53.9 | 4.912 |
| cells/postcard/zstddict | 1.073 | 3.805 | 0.109 | 6.100 | 55.0 | 5.153 |
| cells/postcard/zstdstream | 0.789 | 3.458 | 0.104 | 6.002 | 54.9 | 5.115 |
| cells/prost/none | 22.2 | 107.9 | 0.682 | 350.2 | 473.1 | 149.7 |
| cells/prost/zstd1 | 1.060 | 6.601 | 0.117 | 8.126 | 64.2 | 6.912 |
| runs/bincode/none | 4.537 | 15.2 | 0.540 | 36.9 | 316.5 | 28.7 |
| runs/bincode/zstd1 | 0.638 | 2.361 | 0.106 | 4.803 | 41.7 | 1.992 |
| runs/msgpack/none | 5.117 | 16.3 | 0.642 | 38.2 | 391.6 | 30.0 |
| runs/msgpack/zstd1 | 0.837 | 2.534 | 0.112 | 4.841 | 62.7 | 2.109 |
| runs/postcard/lz4 | 1.280 | 3.695 | 0.213 | 7.821 | 123.7 | 2.612 |
| runs/postcard/none | 4.036 | 14.3 | 0.442 | 35.6 | 301.9 | 27.3 |
| runs/postcard/zstd1 | 0.593 | 2.323 | 0.112 | 4.866 | 56.8 | 1.954 |
| runs/postcard/zstd3 | 0.582 | 2.313 | 0.104 | 4.868 | 52.5 | 1.858 |
| runs/postcard/zstddict | 0.825 | 1.955 | 0.123 | 4.951 | 56.9 | 2.179 |
| runs/postcard/zstdstream | 0.592 | 2.338 | 0.111 | 4.949 | 56.8 | 1.920 |
| runs/prost/none | 7.122 | 19.4 | 0.730 | 41.5 | 482.2 | 33.6 |
| runs/prost/zstd1 | 0.981 | 3.146 | 0.144 | 5.289 | 68.8 | 2.726 |
| vt/raw/lz4 | 1.854 | 4.779 | 0.223 | 7.694 | 137.9 | 2.534 |
| vt/raw/none | 6.625 | 19.0 | 0.929 | 37.2 | 618.2 | 27.4 |
| vt/raw/zstd1 | 0.662 | 2.520 | 0.140 | 4.525 | 72.0 | 1.763 |
| vt/raw/zstd3 | 0.751 | 2.578 | 0.148 | 4.674 | 66.6 | 1.771 |
| vt/raw/zstddict | 1.063 | 2.697 | 0.146 | 5.035 | 72.7 | 2.126 |
| vt/raw/zstdstream | 0.581 | 2.601 | 0.139 | 4.533 | 72.0 | 1.783 |

### wire_kb (KB)

| candidate | w2-plain | w3-buildlog | w4-vim | w5-monitor | w6-fire | w7-unicode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| cells/bincode/none | 227.2 | 156.6 | 1283 | 19581 | 1275 | 46.6 |
| cells/bincode/zstd1 | 13.9 | 11.0 | 118.6 | 1227 | 106.4 | 4.242 |
| cells/msgpack/none | 279.6 | 194.4 | 1603 | 24071 | 1762 | 57.1 |
| cells/msgpack/zstd1 | 15.6 | 11.3 | 121.2 | 1166 | 232.8 | 4.475 |
| cells/postcard/lz4 | 30.3 | 20.2 | 230.9 | 4174 | 506.8 | 7.187 |
| cells/postcard/none | 202.7 | 142.8 | 1176 | 17524 | 1397 | 43.4 |
| cells/postcard/zstd1 | 13.4 | 10.7 | 107.9 | 1216 | 205.9 | 4.317 |
| cells/postcard/zstd3 | 15.1 | 10.6 | 107.1 | 1174 | 202.1 | 4.032 |
| cells/postcard/zstddict | 16.4 | 13.7 | 110.6 | 1229 | 205.9 | 4.955 |
| cells/postcard/zstdstream | 10.7 | 7.177 | 65.8 | 1092 | 202.1 | 3.139 |
| cells/prost/none | 258.1 | 190.1 | 1474 | 22010 | 1762 | 57.3 |
| cells/prost/zstd1 | 15.7 | 14.5 | 119.1 | 1371 | 238.7 | 4.584 |
| runs/bincode/none | 57.6 | 26.7 | 186.3 | 2894 | 1184 | 10.7 |
| runs/bincode/zstd1 | 14.0 | 7.192 | 62.7 | 927.3 | 154.2 | 1.760 |
| runs/msgpack/none | 64.4 | 28.5 | 198.3 | 3040 | 1464 | 11.2 |
| runs/msgpack/zstd1 | 16.1 | 7.562 | 66.2 | 945.0 | 237.0 | 1.812 |
| runs/postcard/lz4 | 18.5 | 9.457 | 91.2 | 1585 | 468.1 | 2.020 |
| runs/postcard/none | 51.1 | 25.1 | 181.3 | 2750 | 1129 | 10.2 |
| runs/postcard/zstd1 | 13.3 | 6.892 | 61.6 | 912.0 | 215.8 | 1.715 |
| runs/postcard/zstd3 | 13.3 | 6.816 | 59.5 | 931.9 | 197.4 | 1.681 |
| runs/postcard/zstddict | 11.9 | 2.429 | 70.8 | 976.9 | 214.1 | 1.806 |
| runs/postcard/zstdstream | 12.9 | 4.311 | 29.3 | 828.9 | 211.3 | 1.147 |
| runs/prost/none | 70.6 | 31.1 | 205.6 | 3151 | 1797 | 11.8 |
| runs/prost/zstd1 | 18.6 | 8.396 | 72.0 | 993.6 | 261.7 | 2.064 |
| vt/raw/lz4 | 29.0 | 11.3 | 96.0 | 1588 | 516.3 | 1.920 |
| vt/raw/none | 83.7 | 33.1 | 223.4 | 3152 | 2312 | 10.4 |
| vt/raw/zstd1 | 10.0 | 7.287 | 58.8 | 886.3 | 271.7 | 1.564 |
| vt/raw/zstd3 | 10.8 | 7.140 | 57.3 | 916.6 | 250.2 | 1.537 |
| vt/raw/zstddict | 15.9 | 4.761 | 70.4 | 986.8 | 273.3 | 1.648 |
| vt/raw/zstdstream | 17.2 | 4.933 | 29.3 | 799.9 | 268.2 | 1.074 |

### wire_ratio (x)

| candidate | w2-plain | w3-buildlog | w4-vim | w5-monitor | w6-fire | w7-unicode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| cells/bincode/none | 0.001 | 0.006 | 7.839 | 7.159 | 0.022 | 0.010 |
| cells/bincode/zstd1 | 0.000 | 0.000 | 0.725 | 0.449 | 0.002 | 0.001 |
| cells/msgpack/none | 0.001 | 0.007 | 9.794 | 8.801 | 0.031 | 0.012 |
| cells/msgpack/zstd1 | 0.000 | 0.000 | 0.741 | 0.426 | 0.004 | 0.001 |
| cells/postcard/lz4 | 0.000 | 0.001 | 1.411 | 1.526 | 0.009 | 0.002 |
| cells/postcard/none | 0.001 | 0.005 | 7.188 | 6.407 | 0.024 | 0.009 |
| cells/postcard/zstd1 | 0.000 | 0.000 | 0.659 | 0.444 | 0.004 | 0.001 |
| cells/postcard/zstd3 | 0.000 | 0.000 | 0.655 | 0.429 | 0.004 | 0.001 |
| cells/postcard/zstddict | 0.000 | 0.001 | 0.676 | 0.449 | 0.004 | 0.001 |
| cells/postcard/zstdstream | 0.000 | 0.000 | 0.402 | 0.399 | 0.004 | 0.001 |
| cells/prost/none | 0.001 | 0.007 | 9.009 | 8.047 | 0.031 | 0.012 |
| cells/prost/zstd1 | 0.000 | 0.001 | 0.728 | 0.501 | 0.004 | 0.001 |
| runs/bincode/none | 0.000 | 0.001 | 1.139 | 1.058 | 0.021 | 0.002 |
| runs/bincode/zstd1 | 0.000 | 0.000 | 0.383 | 0.339 | 0.003 | 0.000 |
| runs/msgpack/none | 0.000 | 0.001 | 1.212 | 1.111 | 0.025 | 0.002 |
| runs/msgpack/zstd1 | 0.000 | 0.000 | 0.405 | 0.346 | 0.004 | 0.000 |
| runs/postcard/lz4 | 0.000 | 0.000 | 0.557 | 0.579 | 0.008 | 0.000 |
| runs/postcard/none | 0.000 | 0.001 | 1.108 | 1.006 | 0.020 | 0.002 |
| runs/postcard/zstd1 | 0.000 | 0.000 | 0.376 | 0.333 | 0.004 | 0.000 |
| runs/postcard/zstd3 | 0.000 | 0.000 | 0.364 | 0.341 | 0.003 | 0.000 |
| runs/postcard/zstddict | 0.000 | 0.000 | 0.433 | 0.357 | 0.004 | 0.000 |
| runs/postcard/zstdstream | 0.000 | 0.000 | 0.179 | 0.303 | 0.004 | 0.000 |
| runs/prost/none | 0.000 | 0.001 | 1.257 | 1.152 | 0.031 | 0.003 |
| runs/prost/zstd1 | 0.000 | 0.000 | 0.440 | 0.363 | 0.005 | 0.000 |
| vt/raw/lz4 | 0.000 | 0.000 | 0.586 | 0.581 | 0.009 | 0.000 |
| vt/raw/none | 0.000 | 0.001 | 1.365 | 1.152 | 0.040 | 0.002 |
| vt/raw/zstd1 | 0.000 | 0.000 | 0.359 | 0.324 | 0.005 | 0.000 |
| vt/raw/zstd3 | 0.000 | 0.000 | 0.350 | 0.335 | 0.004 | 0.000 |
| vt/raw/zstddict | 0.000 | 0.000 | 0.430 | 0.361 | 0.005 | 0.000 |
| vt/raw/zstdstream | 0.000 | 0.000 | 0.179 | 0.292 | 0.005 | 0.000 |

## BEFORE
```
:; wrk -t1 -c50 -d10 http://127.0.0.1:3000
Running 10s test @ http://127.0.0.1:3000
  1 threads and 50 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency   312.98us   63.19us   1.41ms   69.49%
    Req/Sec   108.27k     6.13k  118.74k    82.00%
  1077353 requests in 10.00s, 90.42MB read
Requests/sec: 107722.14
Transfer/sec:      9.04MB
```
## AFTER
```
:; wrk -t1 -c50 -d10 http://127.0.0.1:3000
Running 10s test @ http://127.0.0.1:3000
  1 threads and 50 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency   315.32us   89.24us   3.60ms   87.94%
    Req/Sec   106.28k     8.43k  115.32k    87.00%
  1057254 requests in 10.00s, 88.73MB read
Requests/sec: 105709.84
Transfer/sec:      8.87MB
```

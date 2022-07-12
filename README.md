# ETF-Trace

`etf-trace` uses the integrated debugging components (especially the ITM, DWT,
CSTF, ETF) in common Cortex-M7 CPUs to capture a high rate burst of debugging
trace data into a RAM buffer. It then reads out the buffer and parses the trace
stream.

While this is likely applicable to all CPUs that share these components, it has
only been tested on STM32H7 processors so far.

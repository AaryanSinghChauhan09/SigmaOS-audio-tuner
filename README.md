    # SigmaOS-audio-tuner

    Audio device tuner (EQ, profiles)

    Part of the **SigmaOS** sovereign operating-system ecosystem — a multi-repository
    distribution modelled on Linux Mint and Omarchy Linux. See the parent
    [`SigmaOS`](https://github.com/AaryanSinghChauhan09/SigmaOS) repository for the
    full component index.

    ## Upstream inspirations
    - https://github.com/omacom/omarchy-audio-tuner

    ## Design principles applied
    - **Separation of Concerns**
- **Abstraction**
- **Defensive Programming**
- **Fail-Fast**
- **KISS**
- **Deterministic Testing**

    ## Usage
    Verbs: `devices, profile, eq-plan, balance`

    ```
    $ sigma-audio devices --input sinks.txt
1  alsa_output.pci-0000_00_1f.3.analog-stereo  RUNNING
$ sigma-audio eq-plan bass-boost
31.25 Hz  +5.5 dB  q=1.20
$ sigma-audio eq-plan vocal --format eqapo
$ sigma-audio balance -50
left=1.00 (-0.0 dB)  right=0.50 (-6.0 dB)
$ sigma-audio profile --name desk --sink alsa_output.pci --save desk.json

    ```

    ## Build & test
    ```
    make build
    make test
    ```

    ## Licence
    GNU General Public License v3.0 (FOSS).

# tools

## `oracle_dump.py`

Dumps a `.skp` file as JSON using OpenSKP's Python package, the reference the Rust reader (`crates/mt-skp`) is compared against in `crates/mt-skp/tests/oracle.rs`.

Install the oracle from the same OpenSKP commit the Rust port follows:

```sh
pip install "openskp @ git+https://github.com/iamahsanmehmood/openskp@359d1836c9665484c193635796a54aef004babac#subdirectory=packages/python"
```

Then:

```sh
python3 -I tools/oracle_dump.py tests/corpus/openskp/SU_File.skp
cargo run -p mt-skp --example dump tests/corpus/openskp/SU_File.skp
```

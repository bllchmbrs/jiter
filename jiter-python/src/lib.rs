use pyo3::prelude::*;
use pyo3::pymodule;

use jiter::{python_parse, map_json_error};

#[pyfunction(signature = (data, *, allow_inf_nan=true, cache_strings=true))]
pub fn from_json<'py>(py: Python<'py>, data: &'py [u8], allow_inf_nan: bool, cache_strings: bool) -> PyResult<Bound<'py, PyAny>> {
    let json_bytes = data;
    python_parse(py, json_bytes, allow_inf_nan, cache_strings).map_err(|e| map_json_error(json_bytes, &e))
}

#[pymodule]
fn jiter_python(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(from_json, m)?)?;
    Ok(())
}
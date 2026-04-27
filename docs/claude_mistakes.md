### Forgot a #derive call   
   --> tests/graph_model.rs:258:5
    |
258 |     assert_eq!(d.remove_node(ghost), Err(DiagramError::NodeNotFound(ghost)));
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    |     |
    |     Result<RemovedNode, DiagramError>
    |     Result<RemovedNode, DiagramError>
### Essentially a syntax error:

   --> src/validation/balance.rs:408:17
    |
406 |                 req = required.get(prim).copied().unwrap_or(0),
    |                 ---------------------------------------------- named argument
407 |                 got = supplied.get(prim).copied().unwrap_or(0),
    |                 ---------------------------------------------- named argument
408 |                 d.unsigned_abs(),

### Type error
 error[E0308]: mismatched types
   --> src/validation/balance.rs:197:21
    |
197 |             if *v > 0 {
    |                     ^ expected `&i64`, found integer

### Type annotation missing
 error[E0283]: type annotations needed
   --> src/undo/commands.rs:686:22
    |
686 |             .map_err(|e| format!("{e}").into())?;
    |                      ^^^                ---- type must be known at this point   |   |                 ^^^^^^^^^^^^^^^^ positional arguments must be before named arguments

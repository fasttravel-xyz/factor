use std::any::Any;
use uuid::Uuid;

/// Trait to convert a trait object to the Any super trait.
pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
}
impl<T> AsAny for T
where
    T: 'static, // + Sized,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Entity UUID for the crate.
pub type EntityId = u128; // [todo] distributed id-generation (e.g. sonyflake/snowflake)
/// Generate Entity Id.
pub fn generate_entity_id() -> Result<EntityId, ()> {
    Ok(Uuid::new_v4().as_u128()) // return Result in case error handling required in future
}

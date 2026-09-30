//! Owned mathematical values. No engine references escape into Lua.
use fyrox::core::algebra::{Matrix4, Point3, UnitQuaternion, Vector3};
use mlua::{AnyUserData, Lua, UserData, UserDataFields, UserDataMethods};

#[derive(Clone, Copy)]
pub struct LuaMatrix(pub Matrix4<f32>);
impl UserData for LuaMatrix {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("multiply", |_, this, rhs: AnyUserData| {
            Ok(Self(this.0 * rhs.borrow::<Self>()?.0))
        });
        m.add_method("transform_point", |_, this, (x, y, z): (f32, f32, f32)| {
            let p = this.0.transform_point(&Point3::new(x, y, z));
            Ok((p.x, p.y, p.z))
        });
        m.add_method("get", |_, this, (r, c): (usize, usize)| {
            if !(1..=4).contains(&r) || !(1..=4).contains(&c) {
                return Err(mlua::Error::runtime("matrix indices must be 1..4"));
            }
            Ok(this.0[(r - 1, c - 1)])
        });
    }
}
#[derive(Clone, Copy)]
pub struct LuaQuaternion(pub UnitQuaternion<f32>);
impl UserData for LuaQuaternion {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("x", |_, s| Ok(s.0.i));
        f.add_field_method_get("y", |_, s| Ok(s.0.j));
        f.add_field_method_get("z", |_, s| Ok(s.0.k));
        f.add_field_method_get("w", |_, s| Ok(s.0.w));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("multiply", |_, s, rhs: AnyUserData| {
            Ok(Self(s.0 * rhs.borrow::<Self>()?.0))
        });
        m.add_method("matrix", |_, s, ()| Ok(LuaMatrix(s.0.to_homogeneous())));
        m.add_method("rotate", |_, s, (x, y, z): (f32, f32, f32)| {
            let v = s.0 * Vector3::new(x, y, z);
            Ok((v.x, v.y, v.z))
        });
    }
}
#[derive(Clone, Copy)]
pub struct LuaRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}
impl UserData for LuaRect {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("x", |_, s| Ok(s.x));
        f.add_field_method_get("y", |_, s| Ok(s.y));
        f.add_field_method_get("width", |_, s| Ok(s.width));
        f.add_field_method_get("height", |_, s| Ok(s.height));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("contains", |_, s, (x, y): (f32, f32)| {
            Ok(x >= s.x && y >= s.y && x <= s.x + s.width && y <= s.y + s.height)
        });
    }
}
pub fn register(lua: &Lua) -> mlua::Result<()> {
    let matrix = lua.create_table()?;
    matrix.set(
        "identity",
        lua.create_function(|_, ()| Ok(LuaMatrix(Matrix4::identity())))?,
    )?;
    matrix.set(
        "translation",
        lua.create_function(|_, (x, y, z): (f32, f32, f32)| {
            Ok(LuaMatrix(Matrix4::new_translation(&Vector3::new(x, y, z))))
        })?,
    )?;
    matrix.set(
        "scale",
        lua.create_function(|_, (x, y, z): (f32, f32, f32)| {
            Ok(LuaMatrix(Matrix4::new_nonuniform_scaling(&Vector3::new(
                x, y, z,
            ))))
        })?,
    )?;
    lua.globals().set("Matrix4", matrix)?;
    let q = lua.create_table()?;
    q.set(
        "from_euler",
        lua.create_function(|_, (x, y, z): (f32, f32, f32)| {
            Ok(LuaQuaternion(UnitQuaternion::from_euler_angles(x, y, z)))
        })?,
    )?;
    lua.globals().set("Quaternion", q)?;
    let rect = lua.create_table()?;
    rect.set(
        "new",
        lua.create_function(|_, (x, y, width, height): (f32, f32, f32, f32)| {
            if ![x, y, width, height].iter().all(|x| x.is_finite()) || width < 0.0 || height < 0.0 {
                return Err(mlua::Error::runtime("invalid rectangle"));
            }
            Ok(LuaRect {
                x,
                y,
                width,
                height,
            })
        })?,
    )?;
    lua.globals().set("Rect", rect)
}

#[cfg(test)]
mod tests {
    #[test]
    fn owned_math_uses_real_engine_algebra_and_rejects_invalid_access() {
        let lua = mlua::Lua::new();
        super::register(&lua).unwrap();
        lua.load(
            r#"
          local m=Matrix4.translation(1,2,3):multiply(Matrix4.scale(2,3,4))
          local x,y,z=m:transform_point(2,3,4)
          assert(x==5 and y==11 and z==19)
          local a,b,c=Quaternion.from_euler(0,0,0):rotate(1,2,3)
          assert(a==1 and b==2 and c==3)
          assert(Rect.new(0,0,10,20):contains(5,6))
          assert(not pcall(function() return m:get(0,1) end))
          assert(not pcall(function() return Rect.new(0,0,-1,1) end))
        "#,
        )
        .exec()
        .unwrap();
    }
}

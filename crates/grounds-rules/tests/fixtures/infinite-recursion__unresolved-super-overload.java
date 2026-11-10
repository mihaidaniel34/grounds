// grounds: no-findings
import com.baomidou.mybatisplus.core.conditions.query.LambdaQueryWrapper;
import com.baomidou.mybatisplus.core.conditions.query.QueryWrapper;
import com.baomidou.mybatisplus.core.toolkit.support.SFunction;
import com.github.yulichang.base.MPJBaseMapper;
import java.util.List;
// `selectList(Wrapper)` is declared in the unresolved MyBatis `BaseMapper`;
// the calls below target it, not the enclosing method
interface BaseMapperX<T> extends MPJBaseMapper<T> {
    default List<T> selectList() { return selectList(new QueryWrapper<>()); }
    default List<T> selectList(String field, Object value) { return selectList(new QueryWrapper<T>().eq(field, value)); }
}
class LambdaQueryWrapperX<T> extends LambdaQueryWrapper<T> {
    public LambdaQueryWrapperX<T> likeIfPresent(SFunction<T, ?> column, String val) { return this; }
    public LambdaQueryWrapperX<T> eqIfPresent(SFunction<T, ?> column, Object val) { return this; }
}
class Dept {
    String getName() { return null; }
    Integer getStatus() { return null; }
}
class DeptListReqVO {
    String getName() { return null; }
    Integer getStatus() { return null; }
}
interface DeptMapper extends BaseMapperX<Dept> {
    default List<Dept> selectList(DeptListReqVO reqVO) {
        return selectList(new LambdaQueryWrapperX<Dept>()
                .likeIfPresent(Dept::getName, reqVO.getName())
                .eqIfPresent(Dept::getStatus, reqVO.getStatus()));
    }
    // a known argument type that is a subtype of ours: an inherited
    // `selectOne(String)` would be more specific
    default Dept selectOne(CharSequence name) { return selectOne(name.toString()); }
    // a varargs element may match an inherited fixed-arity overload
    default Dept selectAny(Object... keys) { return selectAny("id"); }
}

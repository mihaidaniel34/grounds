package app;
import org.mapstruct.Mapper;
import org.mapstruct.MappingConstants;
import org.springframework.stereotype.Service;

@Mapper(componentModel = "spring") interface IfaceStringMapper {}
@Mapper(componentModel = MappingConstants.ComponentModel.SPRING) interface IfaceConstMapper {}
@Mapper(componentModel = "spring") abstract class AbstractStringMapper {}
@Mapper(componentModel = MappingConstants.ComponentModel.SPRING) abstract class AbstractConstMapper {}
@Mapper interface IfaceNoModelMapper {}
@Mapper abstract class AbstractNoModelMapper {}
@Mapper(componentModel = "default") interface IfaceDefaultMapper {}
@Mapper(componentModel = MappingConstants.ComponentModel.DEFAULT) interface IfaceDefaultConstMapper {}
@Mapper(componentModel = "default") abstract class AbstractDefaultMapper {}
@Mapper(componentModel = MappingConstants.ComponentModel.DEFAULT) abstract class AbstractDefaultConstMapper {}

@Service class UsesIfaceString { UsesIfaceString(IfaceStringMapper m) {} }
@Service class UsesIfaceConst { UsesIfaceConst(IfaceConstMapper m) {} }
@Service class UsesAbstractString { UsesAbstractString(AbstractStringMapper m) {} }
@Service class UsesAbstractConst { UsesAbstractConst(AbstractConstMapper m) {} }
@Service class UsesIfaceNoModel { UsesIfaceNoModel(IfaceNoModelMapper m) {} }
@Service class UsesAbstractNoModel { UsesAbstractNoModel(AbstractNoModelMapper m) {} }
// a default-model mapper is obtained with Mappers.getMapper, not injected
@Service class UsesIfaceDefault { UsesIfaceDefault(IfaceDefaultMapper m) {} } // expect
@Service class UsesIfaceDefaultConst { UsesIfaceDefaultConst(IfaceDefaultConstMapper m) {} } // expect
@Service class UsesAbstractDefault { UsesAbstractDefault(AbstractDefaultMapper m) {} } // expect
@Service class UsesAbstractDefaultConst { UsesAbstractDefaultConst(AbstractDefaultConstMapper m) {} } // expect

@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp {}

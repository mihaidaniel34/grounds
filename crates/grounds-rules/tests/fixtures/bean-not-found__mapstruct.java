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
// constants the index cannot evaluate: only one naming DEFAULT itself is the default model
@Mapper(componentModel = Models.DEFAULT_SPRING) abstract class AbstractUnknownSpringConstMapper {}
@Mapper(componentModel = Unknown.ComponentModel.DEFAULT) abstract class AbstractUnknownDefaultConstMapper {}
@Mapper(componentModel = (MappingConstants.ComponentModel.DEFAULT)) abstract class AbstractParenDefaultMapper {}
@Mapper(componentModel = (Flags.ON ? Models.SPRING : Models.DEFAULT)) abstract class AbstractUnknownChoiceMapper {}
@Mapper(componentModel = (Unknown . Component$Model . DEFAULT)) abstract class AbstractSpacedDefaultMapper {}

@Service class UsesIfaceString { UsesIfaceString(IfaceStringMapper m) {} }
@Service class UsesIfaceConst { UsesIfaceConst(IfaceConstMapper m) {} }
@Service class UsesAbstractString { UsesAbstractString(AbstractStringMapper m) {} }
@Service class UsesAbstractConst { UsesAbstractConst(AbstractConstMapper m) {} }
@Service class UsesIfaceNoModel { UsesIfaceNoModel(IfaceNoModelMapper m) {} }
@Service class UsesAbstractNoModel { UsesAbstractNoModel(AbstractNoModelMapper m) {} }
@Service class UsesAbstractUnknownSpringConst { UsesAbstractUnknownSpringConst(AbstractUnknownSpringConstMapper m) {} }
@Service class UsesAbstractUnknownChoice { UsesAbstractUnknownChoice(AbstractUnknownChoiceMapper m) {} }
// a default-model mapper is obtained with Mappers.getMapper, not injected
@Service class UsesIfaceDefault { UsesIfaceDefault(IfaceDefaultMapper m) {} } // expect
@Service class UsesIfaceDefaultConst { UsesIfaceDefaultConst(IfaceDefaultConstMapper m) {} } // expect
@Service class UsesAbstractDefault { UsesAbstractDefault(AbstractDefaultMapper m) {} } // expect
@Service class UsesAbstractDefaultConst { UsesAbstractDefaultConst(AbstractDefaultConstMapper m) {} } // expect
@Service class UsesAbstractUnknownDefaultConst { UsesAbstractUnknownDefaultConst(AbstractUnknownDefaultConstMapper m) {} } // expect
@Service class UsesAbstractParenDefault { UsesAbstractParenDefault(AbstractParenDefaultMapper m) {} } // expect
@Service class UsesAbstractSpacedDefault { UsesAbstractSpacedDefault(AbstractSpacedDefaultMapper m) {} } // expect

@org.springframework.boot.autoconfigure.SpringBootApplication class FixtureApp {}

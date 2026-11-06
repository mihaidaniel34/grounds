import org.mapstruct.Mapper;
import org.mapstruct.MappingConstants;
import org.springframework.stereotype.Component;

// MapStruct generates and registers an implementation for these
@Mapper(componentModel = "spring") @Component abstract class AbstractStringModelMapper {}
@Mapper(componentModel = MappingConstants.ComponentModel.SPRING) @Component abstract class AbstractConstModelMapper {}
@Mapper(componentModel = MappingConstants.ComponentModel.SPRING) @Component interface InterfaceConstModelMapper {}
// the component model may come from the mapstruct.defaultComponentModel compiler option
@Mapper @Component abstract class AbstractNoModelMapper {}

// the default component model registers nothing
@Mapper(componentModel = "default") @Component abstract class AbstractDefaultModelMapper {} // expect
@Mapper(componentModel = MappingConstants.ComponentModel.DEFAULT) @Component abstract class AbstractDefaultConstMapper {} // expect

package org.springframework.data.repository; import java.util.Optional; public interface CrudRepository<T, ID> extends Repository<T, ID> { <S extends T> S save(S e); Optional<T> findById(ID id); }

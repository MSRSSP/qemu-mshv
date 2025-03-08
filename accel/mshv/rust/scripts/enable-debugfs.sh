echo 1 > /sys/kernel/tracing/events/mshv/mshv_create_vp/enable
echo 1 > /sys/kernel/tracing/events/mshv/mshv_vp_release/enable 
echo 1 > /sys/kernel/tracing/events/mshv/mshv_hvcall_initialize_partition/enable 
echo 1 > /sys/kernel/tracing/events/mshv/mshv_hvcall_delete_partition/enable 
echo 1 > /sys/kernel/tracing/tracing_on

# /sys/kernel/tracing/trace
# echo 0 > /sys/kernel/tracing/tracing_on
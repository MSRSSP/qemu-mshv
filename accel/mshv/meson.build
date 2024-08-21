mshv_ss = ss.source_set()
mshv_ss.add(when: mshv, if_true: files(
  'mshv-all.c',
  'irq.c'
))

specific_ss.add_all(when: 'CONFIG_MSHV', if_true: mshv_ss)

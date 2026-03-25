; Python message queue detection for CX
; Captures: @mq.topic, @mq.publish, @mq.subscribe

; kafka-python: producer.send('topic', value)
(call
  function: (attribute
    attribute: (identifier) @_method)
  arguments: (argument_list
    (string) @mq.topic)
  (#eq? @_method "send")) @mq.publish

; KafkaConsumer('topic')
(call
  function: (identifier) @_fn
  arguments: (argument_list
    (string) @mq.topic)
  (#eq? @_fn "KafkaConsumer")) @mq.subscribe

; pika: channel.basic_publish(routing_key='queue', ...)
(call
  function: (attribute
    attribute: (identifier) @_method)
  arguments: (argument_list
    (keyword_argument
      name: (identifier) @_kwarg
      value: (string) @mq.topic))
  (#eq? @_method "basic_publish")
  (#eq? @_kwarg "routing_key")) @mq.publish

; pika: channel.basic_consume('queue', callback)
(call
  function: (attribute
    attribute: (identifier) @_method)
  arguments: (argument_list
    (string) @mq.topic)
  (#eq? @_method "basic_consume")) @mq.subscribe

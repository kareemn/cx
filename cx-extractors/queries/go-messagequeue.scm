; Go message queue detection for CX
; Captures: @mq.topic, @mq.publish, @mq.subscribe

; Publish patterns: producer.Publish("topic", msg), nats.Publish("subject", data)
(call_expression
  function: (selector_expression
    field: (field_identifier) @_method)
  arguments: (argument_list
    (interpreted_string_literal) @mq.topic)
  (#match? @_method "^(Publish|Send|SendMessage|Produce)$")) @mq.publish

; Subscribe patterns: consumer.Subscribe("topic", handler), nats.Subscribe("subject", handler)
(call_expression
  function: (selector_expression
    field: (field_identifier) @_method)
  arguments: (argument_list
    (interpreted_string_literal) @mq.topic)
  (#match? @_method "^(Subscribe|Consume|QueueSubscribe|ChanSubscribe)$")) @mq.subscribe

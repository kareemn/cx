; TypeScript/JavaScript message queue detection for CX
; Captures: @mq.topic, @mq.publish, @mq.subscribe

; kafkajs: producer.send({ topic: 'x', messages: [...] })
(call_expression
  function: (member_expression
    property: (property_identifier) @_method)
  arguments: (arguments
    (object
      (pair
        key: (property_identifier) @_key
        value: (string) @mq.topic)))
  (#eq? @_method "send")
  (#eq? @_key "topic")) @mq.publish

; kafkajs: consumer.subscribe({ topic: 'x' })
(call_expression
  function: (member_expression
    property: (property_identifier) @_method)
  arguments: (arguments
    (object
      (pair
        key: (property_identifier) @_key
        value: (string) @mq.topic)))
  (#eq? @_method "subscribe")
  (#eq? @_key "topic")) @mq.subscribe

; amqplib: channel.sendToQueue('queue', buffer)
(call_expression
  function: (member_expression
    property: (property_identifier) @_method)
  arguments: (arguments
    (string) @mq.topic)
  (#match? @_method "^(publish|sendToQueue)$")) @mq.publish

; amqplib: channel.consume('queue', handler)
(call_expression
  function: (member_expression
    property: (property_identifier) @_method)
  arguments: (arguments
    (string) @mq.topic)
  (#eq? @_method "consume")) @mq.subscribe
